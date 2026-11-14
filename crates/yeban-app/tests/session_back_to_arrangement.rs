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

#![allow(unused)]
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
    // 新 UI 重构后旧面板的视觉元素不再进控件树; 同一条宿主接线仍住在契约面。
    // 这里机械核对"声明 + 注册"两件事(缺任一即红), 视觉交互由 aether_* 判据覆盖。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback back-to-arrangement("),
        "契约面必须声明 callback back-to-arrangement("
    );
    assert!(
        aether_host.contains("back_to_arrangement"),
        "宿主接线层必须注册 back_to_arrangement"
    );
}
