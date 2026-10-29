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
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
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
    (port, view)
}

#[test]
fn clicking_back_to_arrangement_returns_to_the_arrangement_view() {
    let (mut port, view) = live_window();

    // ---- ① 装配态：编曲视图，Session 子树与返回键都不在树里 ----
    assert!(
        port.ui().get_arrangement_view(),
        "装配态必须是编曲视图（默认帧与既有基准一位不变）"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("workspace-arrangement-canvas").is_some(),
        "编曲画布必须在默认视图的运行时树里"
    );
    assert!(
        tree.find_by_id("workspace-session-canvas").is_none(),
        "Session 画布在编曲视图下必须不可见 ⇒ 不在运行时树里"
    );
    let default_frame = port.window().capture().expect("Tier-1 截图");
    let default_pixels = default_frame.pixels().to_vec();

    // ---- 搭舞台：进 Session 视图（产品里由 `toggle-view` / Tab 切换） ----
    port.ui().set_arrangement_view(false);
    // 树快照活着时不能借窗口抓帧（`refresh_tree` 是可借用的句柄），
    // 所以这段断言放进块里，抓帧之后 ② 再重抓一次。
    {
        let tree = port
            .refresh_tree(None)
            .expect("切到 Session 之后必须能重抓");
        assert!(
            tree.find_by_id("workspace-session-canvas").is_some(),
            "Session 画布必须在 Session 视图的运行时树里"
        );
        assert!(
            tree.find_by_id("workspace-arrangement-canvas").is_none(),
            "编曲画布在 Session 视图下必须不可见"
        );
    }
    let session_frame = port.window().capture().expect("Tier-1 截图");
    assert!(
        default_pixels != session_frame.pixels(),
        "前提：两个视图的帧必须不同（否则下面的逐位还原证明不了任何事）"
    );

    // ---- ② 可寻址：返回键在场，角色 / 名称与注册表一致 ----
    let registry = ElementRegistry::from_view(&view);
    let tree = port.refresh_tree(None).expect("重抓（② 要读节点）");
    let node = tree
        .find_by_id(BACK_BUTTON)
        .expect("返回键必须在 Session 视图的运行时树里");
    assert_eq!(
        node.role.as_str(),
        ElementKind::Button.accessible_role(),
        "`{BACK_BUTTON}` 的角色必须与注册表一致"
    );
    let meta = registry
        .get(BACK_BUTTON)
        .unwrap_or_else(|| panic!("`{BACK_BUTTON}` 必须在装配时的注册表里（[UI-TEST-001]）"));
    assert_eq!(meta.kind, ElementKind::Button);
    assert_eq!(
        node.label.as_str(),
        meta.label.as_str(),
        "屏读器读到的名称必须与注册表登记的逐字一致"
    );
    let bounds = node
        .bounds
        .unwrap_or_else(|| panic!("`{BACK_BUTTON}` 必须有几何包围盒"));
    assert!(
        bounds.width > 0 && bounds.height > 0,
        "返回键必须有可见几何: {bounds:?}"
    );

    // ---- ③ 真实点击：属性翻过去，主工作区换回时间轴 ----
    let x = f64::from(bounds.width) / 2.0;
    let y = f64::from(bounds.height) / 2.0;
    port.dispatch_pointer_down(BACK_BUTTON, x, y, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
    assert!(
        port.ui().get_arrangement_view(),
        "点击返回键之后 `arrangement-view` 必须是 true"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("workspace-arrangement-canvas").is_some(),
        "点击之后编曲画布必须回到运行时树里"
    );
    assert!(
        tree.find_by_id("workspace-session-canvas").is_none(),
        "点击之后 Session 画布必须不可见"
    );
    assert!(
        tree.find_by_id(BACK_BUTTON).is_none(),
        "点击之后返回键自己必须不可见（Session 子树整棵换出）"
    );

    // ---- ④ 可见态逐位还原：整帧像素回到装配态默认帧 ----
    let restored = port.window().capture().expect("Tier-1 截图");
    assert!(
        default_pixels == restored.pixels(),
        "返回编曲之后的整帧像素必须与装配态默认帧逐字节相同（视图真的换回去了）"
    );

    // ---- ⑤ 写 true、不是翻转：再按一次不该把用户踢回 Session ----
    port.ui().set_arrangement_view(false);
    port.refresh_tree(None).expect("重抓");
    port.ui().invoke_back_to_arrangement();
    assert!(
        port.ui().get_arrangement_view(),
        "Session 态下直接调用回调必须写 `true`"
    );
    port.ui().invoke_back_to_arrangement();
    assert!(
        port.ui().get_arrangement_view(),
        "已经在编曲视图时再调一次必须仍是 `true`（写 true，⛔ 不是翻转）"
    );

    // ---- ⑥ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let session = std::fs::read_to_string(manifest_dir.join("ui/workspace/session_view.slint"))
        .expect("读 session_view.slint");
    assert!(
        session.contains("callback back-to-arrangement();"),
        "Session 视图必须声明返回编曲回调"
    );
    assert!(
        session.contains("accessible-action-default => { root.back-to-arrangement(); }"),
        "返回键必须有无障碍动作"
    );
    assert!(
        session.contains("clicked => { root.back-to-arrangement(); }"),
        "返回键必须有点击源，且落点与无障碍动作逐字相同"
    );
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    assert!(
        app.contains("callback back-to-arrangement();")
            && app.contains("back-to-arrangement => {")
            && app.contains("root.back-to-arrangement();"),
        "主窗口必须声明回调并把 Session 的点击原样转给宿主"
    );
    let host_src = std::fs::read_to_string(manifest_dir.join("src/host.rs")).expect("读 host.rs");
    assert!(
        host_src.contains("ui.on_back_to_arrangement(")
            && host_src.contains("ui.set_arrangement_view(true);"),
        "宿主必须是唯一的落点，并且写的是 `true`"
    );

    report_line(&format!(
        "[session-back] 返回键几何 {bounds:?}；Session 帧 ≠ 默认帧；点击后属性 false → true，\
         整帧 {} 字节逐位还原；重复调用仍为 true",
        restored.pixels().len()
    ));
}
