//! ⭐ T-FIX-260：场景启动回执在最小画布上也必须看得全。
//!
//! 修前 `session_view.slint` 的回执框从场景列左沿起算、固定 320px 宽，
//! 1024 展开态画布只有 504px ⇒ 框右沿越过根 `clip`，只剩约 112px 可见；
//! `overflow: elide` 再把“事件环满 / 引擎没起来”的长原因截成十来个字符。
//!
//! 本判据不直接读 Slint 的文本排版宽度，而是用**两帧的尾部差异**证明整句真的画出来了：
//! 同一前缀、不同结尾的两条长回执，如果尾部仍被 elide 截掉，渲染像素应完全相同；
//! 只有整句都在框内时，第二帧才会在尾部出现不同字形。
#![allow(unused)]

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::tree::ControlTree;
use yeban_ui_test_port::{Permission, Size, UiTestPort as _};

/// 最小档（1024×700 展开态，画布 504px 宽）—— 三档里回执可用宽度最小的一档。
const WINDOW: Size = Size::new(1024, 700);

/// 宿主真实会写的长原因（`SceneLaunchDisposition::EventQueueFull`，host.rs:807）。
const LONG_REASON: &str = "事件环满：这次启动没有送到引擎（再按一次）";
/// 与 [`LONG_REASON`] 共享到 `（` 为止的同一前缀，只换尾部；若尾部被 elide，两帧应相同。
const DIFFERENT_TAIL: &str = "事件环满：这次启动没有送到引擎（ABCDEFGH）";

fn live_window() -> LivePort<yeban_app::ui::MainWindow> {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let mut port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    port.ui().set_arrangement_view(false);
    port.refresh_tree(None)
        .expect("切到 Session 之后必须能重抓控件树");
    port
}

fn bounds(tree: &ControlTree, id: &str) -> (i32, i32, u32, u32) {
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("控件树必须有 `{id}`"));
    let rect = node.bounds.unwrap_or_else(|| panic!("`{id}` 必须有几何"));
    (rect.x, rect.y, rect.width, rect.height)
}

#[test]
fn a_long_scene_launch_reason_is_not_elided_on_the_smallest_canvas() {
    let port = live_window();
    let root = bounds(port.tree(), "workspace-session-canvas");
    let status = bounds(port.tree(), "session-scene-launch-status");

    assert_eq!(
        status.0,
        root.0 + 8,
        "回执左沿必须贴根左沿 + 8px，实际 {status:?} / 根 {root:?}"
    );
    assert_eq!(
        status.2,
        root.2 - 16,
        "回执宽必须是根宽 - 16px，实际 {status:?} / 根 {root:?}"
    );
    assert!(
        status.0 + status.2 as i32 <= root.0 + root.2 as i32 - 8,
        "回执右沿 {} 必须 ≤ 根右沿 {} - 8px",
        status.0 + status.2 as i32,
        root.0 + root.2 as i32
    );

    port.ui().set_scene_launch_status(LONG_REASON.into());
    let full = port.window().capture().expect("长回执必须能渲染整帧");
    port.ui().set_scene_launch_status(DIFFERENT_TAIL.into());
    let tail_changed = port.window().capture().expect("换尾部之后必须能渲染整帧");

    assert_ne!(
        full.pixels(),
        tail_changed.pixels(),
        "只换回执尾部却得到相同像素 ⇒ 尾部被 elide 截掉；长原因必须在整行内完整可见"
    );
}
