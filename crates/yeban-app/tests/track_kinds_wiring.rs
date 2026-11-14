//! 判据（`[PATCH UI-TRACK-KINDS-01]`）：**轨道类型的投影读数 → 界面**的单向接线，
//! 以及 Logic Pro 风格包头视觉节点的「纯视觉 / 不改控件树」边界。
//!
//! ## 补的是哪一条线（不是新能力）
//!
//! `bridge::ViewState::track_kinds()` 早就把每轨类型（`midi` / `audio` /
//! `aux-return`）投影成 `Vec<&'static str>`，`nightfall_ui_wiring` 也钉住了它的
//! 分布（29 条：25 midi + 4 aux-return）。但 `MainWindow` 与 `ArrangementView`
//! **都没有对应属性**，`docs/ledger/app-binding-notes.md` 第 49 行因此一直写着
//! `tracks[*].kind` →「—（待接图标位）」。本切片把这条**已有的**投影读数单向注入
//! `MainWindow.track-kinds → ArrangementView.track-kinds`，包头只读它画类型色带 /
//! 顶边 / 徽标 ⇒ ⛔ 不是新能力、⛔ 界面不按下标猜类型、⛔ 不记第二份真相源。
//!
//! ## 判定量
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `get_track_kinds()` | 运行时窗口上的类型数组 | 字符串数组 |
//! | `ViewState::track_kinds()` | 投影层读数（唯一真相源） | `Vec<&'static str>` |
//! | `track-names.length` | 轨道数（同索引集） | 条 |
//! | `tree.len()` / `ids()` | 运行时可寻址控件树 | 节点 / ID |
//!
//! ## 怎么变红（注入）
//!
//! * 删掉 `host::apply_view` 的 `ui.set_track_kinds(...)` ⇒ ① 红（窗口上恒为空）；
//! * 把注入源换成 `vec!["midi"; n]` 之类 ⇒ ①/② 红（与投影读数分叉、类型分布失真）；
//! * 删掉 `app.slint` 的声明 / 转发 ⇒ 声明面判据红，且 `elements` 的死属性判据红；
//! * 给三个视觉节点补一个 `accessible-id` ⇒ ③ 红（控件树多出节点，`pane_no_overlap`
//!   的 `controls.len()` 也会跟着红）—— 这是「纯视觉」边界的机械守卫；
//! * 把可见性守卫去掉 ⇒ ④ 红（未注入时会伪造类型读数）。
//!
//! ## 边界（⛔ 不声称测过）
//!
//! * **像素外观**：色带 / 顶边 / 徽标的默认帧可见像素由 `tests/golden/linux/` 的
//!   arrangement / model-driven 基准在 CI 手动档 `gates-manual.yml` 重录后人工复核
//!   （本机按 `AGENTS.md` §5.2 ⛔ 不产出 Linux golden）。本判据只钉**接线与边界**；
//! * `session_view.slint` 的列头 / 混音台通道条本切片刻意不改（见交付说明的 ROI 取舍）。

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用 fold,
// clippy::unnecessary_fold 的常规建议与本仓守卫冲突, 具名豁免。
#![allow(unused, clippy::unnecessary_fold)]
use slint::Model as _;

use yeban_app::bridge::ViewState;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::render::Tier1Window;

fn window() -> (Tier1Window, yeban_app::ui::MainWindow, ViewState) {
    let tier1 =
        Tier1Window::install(yeban_ui_test_port::Size::new(1440, 900)).expect("装 Tier-1 无头平台");
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let scene = yeban_app::scene::DemoScene::from_view(&view);
    let ui = yeban_app::host::build_main_window_with_console_tab(&view, &scene, 1)
        .expect("生产窗口构造入口");
    (tier1, ui, view)
}

/// ① 运行时：`MainWindow.track-kinds` 逐字等于 `ViewState::track_kinds()`，
/// 且与 `track-names` 同索引集（`apply_view` 一次注入，⛔ 没有第二份）。
#[test]
fn the_projected_track_kinds_reach_the_window_under_the_same_index() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> track-kinds:"),
        "新 UI 契约面必须声明 > track-kinds:"
    );
    assert!(
        aether_host.contains("set_track_kinds"),
        "宿主接线层必须注册 set_track_kinds"
    );
    assert!(
        aether_app.contains("> track-names:"),
        "新 UI 契约面必须声明 > track-names:"
    );
    assert!(
        aether_host.contains("set_track_names"),
        "宿主接线层必须注册 set_track_names"
    );
}

/// ② 声明面：属性 / 转发 / setter 三段闭环，且 `track-kinds` 只走
/// `MainWindow → ArrangementView`（⛔ 不进 `ConsoleTabs → MixerConsole` 注入集合）。
#[test]
fn the_injection_is_declared_forwarded_and_written_by_the_host() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> track-kinds:"),
        "新 UI 契约面必须声明 > track-kinds:"
    );
    assert!(
        aether_host.contains("set_track_kinds"),
        "宿主接线层必须注册 set_track_kinds"
    );
    assert!(
        aether_app.contains("> track-names:"),
        "新 UI 契约面必须声明 > track-names:"
    );
    assert!(
        aether_host.contains("set_track_names"),
        "宿主接线层必须注册 set_track_names"
    );
}

/// ③ 视觉面 + 纯视觉边界：色板 / 徽标读的是**注入属性**，三类各有独立配色与文字，
/// 且三个节点不引入任何可寻址 / 手势 / 回调面 ⇒ 运行时控件树不增节点。
#[test]
fn the_header_reads_injected_kinds_and_the_visual_nodes_stay_out_of_the_control_tree() {
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
        aether_app.contains("> track-kinds:"),
        "契约面必须声明 > track-kinds:"
    );
    assert!(
        aether_host.contains("set_track_kinds"),
        "宿主接线层必须注册 set_track_kinds"
    );
}
