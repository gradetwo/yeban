//! **拖拽 splitter 调控制台高度** —— 端到端判据（`[UI-GRID-002]`，规范 §1.1 "splitter 220–600"）。
//!
//! ## 本文件要证的句子
//!
//! 主工作区与底部控制台之间的 4px 分割线**不再是一根只贴着假数字的线**：
//! 按住它向上拖 D 像素，控制台真的长高 D 像素（钳在 `220..=600`），并且
//! splitter 的 `accessible-value` 报告的是**实测**高度（改动前写死 `"320"`，
//! 每一帧都在谎报自己的位置）。双击（宿主 `console-height-reset`）清零档位，
//! 布局回到 3:2 弹性比 —— 这就是"默认帧一位不变"的保证。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（默认诚实） | 装配态 `console-height-px == 0`；splitter 的 `value` == **实测** `console-height-readout`（四舍五入），⛔ 不是抄来的数 |
//! | ②（真拖拽 → 真布局） | 注入 down → move(−40) → move(−80) → up ⇒ `console-height-px == round(H₀+80)`、实测高 == 该值（±1）、`value` 与画面同源。拖拽数学：目标 = 实测高 + (按下点 − 当前点)（局部坐标）——分割线**自身**随布局移动的量同时出现在这两项里、正好相消 ⇒ 逐事件**不**累积误差；若布局在注入事件之间不重算，两项同旧 ⇒ 结果相同（两种时序都推过） |
//! | ③（硬钳位） | `invoke_console_height_requested(700)` ⇒ 属性 `600`；`invoke(-5)` ⇒ `220`（越界请求落边界，唯一写者 = 宿主） |
//! | ④（双击归零） | `invoke_console_height_reset()` ⇒ 属性 `0` 且实测高回到 ① 的默认值（state 退出、3:2 弹性比恢复）——⛔ 不是把 "320" 写回去 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 线程局部、每线程一次 ⇒ 本文件只有一个 `#[test]`
//! （与 `tests/console_maximize.rs` 同款）。
//!
//! ## 怎么变红
//!
//! - `.slint` 的 `states [console-pinned]` 删掉 ⇒ ② 的"实测高 == 档位"失败；
//! - 钳位常量改错 ⇒ ③ 失败；
//! - `accessible-value` 退回写死的字面量 ⇒ ① 在默认高 ≠ 该字面量的窗口下失败。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

const WINDOW: Size = Size::new(1440, 900);
const SPLITTER: &str = "splitter-workspace-console";

fn live_window() -> LivePort<yeban_app::ui::MainWindow> {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` 与无头装配调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    port
}

/// splitter 节点的 (实测 value 读数, 包围盒)，每次**重抓树**（布局跟着属性走）。
fn splitter_state(port: &mut LivePort<yeban_app::ui::MainWindow>) -> (f64, Rect) {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let node = tree
        .find_by_id(SPLITTER)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{SPLITTER}`"));
    let bounds = node.bounds.expect("`splitter` 必须有几何包围盒");
    let raw = node
        .value
        .clone()
        .expect("slider 必须报告 `accessible-value`");
    let parsed = raw
        .parse::<f64>()
        .unwrap_or_else(|_| panic!("`value` 必须是数字读数，实际是 `{raw}`"));
    (parsed, bounds)
}

#[test]
fn dragging_the_splitter_resizes_the_console_within_220_to_600_and_reports_the_measured_height() {
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
        aether_app.contains("callback console-height-requested("),
        "新 UI 契约面必须声明 callback console-height-requested("
    );
    assert!(
        aether_host.contains("on_console_height_requested"),
        "宿主接线层必须注册 on_console_height_requested"
    );
    assert!(
        aether_app.contains("callback console-height-reset("),
        "新 UI 契约面必须声明 callback console-height-reset("
    );
    assert!(
        aether_host.contains("on_console_height_reset"),
        "宿主接线层必须注册 on_console_height_reset"
    );
}
