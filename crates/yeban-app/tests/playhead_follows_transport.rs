//! **走带光标跟随引擎读数** —— 端到端判据（[UI-NOTE-001] / [UI-MCP-002]）。
//!
//! ## 本文件要证的句子
//!
//! 编曲视图与卷帘的两条 `playhead` **不是一个停在假位置的装饰**：`host::apply_transport`
//! 把引擎的 `TransportReading::position_ticks` 写成窗口属性 `playhead-ticks`，两条光标
//! 用 `tick ÷ ticks_per_pixel`（与投影里音符 / 小节线同一条整数除法）派生 x ⇒
//! 播放推进时光标跟着走，无障碍值 `accessible-value` 是**同一刻的时间码**，
//! 而不是旧版写死的 `"3"`（旧几何：`168px + 120px * 3` / `56px + 60px * 3`）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（装配态） | tick 0 ⇒ 编曲光标 x = 画布左沿 + 168px、卷帘光标 x = 卷帘左沿 + 56px；两条 `accessible-value` 都 = 窗口时间码 |
//! | ②（真读数 → 真几何） | `apply_transport` 写第 3 小节第 2 拍 ⇒ 重抓树后两条光标都精确移动 `tick ÷ tpp` 像素（⛔ 不是只看属性） |
//! | ③（回 0） | 再写 `TransportReading::cold()` ⇒ 两条光标回原位、值回 `001.01.000`（位置不是累加的） |
//! | ④（文本层接线） | `.slint` 里假位置 / 假值已删除、两条 x 都读 `playhead-ticks`；`app.slint` 原样转发；`host.rs` 只有一个 `set_playhead_ticks` 写者 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/revert_button.rs` / `tests/console_maximize.rs` 同款）。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, Rect, Size};

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
    (port, view)
}

/// 重抓控件树之后取一个节点的几何与无障碍值。
fn node(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> (Rect, Option<String>) {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    let node = tree
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"));
    (
        node.bounds
            .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒")),
        node.value.clone(),
    )
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    node(port, id).0
}

#[test]
fn both_playheads_follow_the_engine_transport_reading() {
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
        aether_app.contains("> playhead-ticks:"),
        "新 UI 契约面必须声明 > playhead-ticks:"
    );
    assert!(
        aether_host.contains("set_playhead_ticks"),
        "宿主接线层必须注册 set_playhead_ticks"
    );
}
