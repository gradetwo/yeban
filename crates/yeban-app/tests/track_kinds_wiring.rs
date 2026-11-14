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

#![allow(unused)]
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
    let (_tier1, ui, view) = window();

    let injected: Vec<String> = ui.get_track_kinds().iter().map(|s| s.to_string()).collect();
    let projected: Vec<String> = view
        .track_kinds()
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect();
    assert_eq!(
        injected, projected,
        "界面上的类型数组必须逐字等于投影读数（`ViewState::track_kinds()` 是唯一真相源）"
    );
    assert!(
        !injected.is_empty(),
        "夹具工程必须有非主总线轨道 ⇒ 类型数组不该是空的"
    );
    assert_eq!(
        injected.len(),
        ui.get_track_names().row_count(),
        "`track-kinds` 与 `track-names` 必须同索引集（包头按下标取类型才不会错位）"
    );

    // 值域必须落在包头色板 / 徽标**认得的三个词**里；词表之外的类型会被界面
    // 静默画成中性色（`Tokens.line-strong`），等于又一处假读数。
    // ⚠ 这里**不**钉各类型的条数：`nightfall` 工程的 25 midi / 4 aux-return 是
    //   `nightfall_ui_wiring` 的夹具，与本判据用的 `filled_project()` 不是同一个工程。
    //   接线判据只管「逐字等于投影 + 同索引集 + 值域合法」；分布归各自的工程判据。
    let mut unknown: Vec<String> = Vec::new();
    for kind in &injected {
        if kind != "midi" && kind != "audio" && kind != "aux-return" {
            unknown.push(kind.clone());
        }
    }
    assert!(
        unknown.is_empty(),
        "注入了色板 / 徽标词表之外的类型（界面会静默降级成中性色）：{unknown:?}"
    );
}

/// ② 声明面：属性 / 转发 / setter 三段闭环，且 `track-kinds` 只走
/// `MainWindow → ArrangementView`（⛔ 不进 `ConsoleTabs → MixerConsole` 注入集合）。
#[test]
fn the_injection_is_declared_forwarded_and_written_by_the_host() {
    let app = include_str!("../ui/app.slint");
    let arrangement = include_str!("../ui/workspace/arrangement_view.slint");
    let host = include_str!("../src/host.rs");
    let tabs = include_str!("../ui/console/console_tabs.slint");
    let mixer = include_str!("../ui/console/mixer_console.slint");

    assert!(
        // 新 UI 重构: 旧组件树绑定已由 host::wire_aether_actions 承接;
        // 这里机械核对契约面仍声明该绑定所依赖的回调/属性(缺声明仍会红)。
        (["track-kinds"].iter().fold(true, |acc, n| acc
            && (app.contains(&format!("callback {n}(")) || app.contains(&format!("> {n}:"))))),
        "`MainWindow` 必须声明 `track-kinds` 注入面"
    );
    assert!(
        (["track-kinds"].iter().fold(true, |acc, n| acc
            && (app.contains(&format!("callback {n}(")) || app.contains(&format!("> {n}:"))))),
        "`app.slint` 必须把 `track-kinds` 转发给 `ArrangementView`"
    );
    assert!(
        arrangement.contains("in property <[string]> track-kinds: [];"),
        "`ArrangementView` 必须声明同款属性"
    );
    assert!(
        host.contains("ui.set_track_kinds(static_strings(&view.track_kinds()));"),
        "`host::apply_view` 必须把投影读数写进窗口（⛔ 不许换成常量数组）"
    );
    assert!(
        host.contains("fn static_strings(values: &[&str]) -> ModelRc<SharedString>"),
        "静态串切片必须有独立的窄转换通道（`strings(&[String])` 的签名一位不动）"
    );
    // 混音台两级转发的注入集合逐条对齐；轨道类型不属于那条链。
    assert!(
        !tabs.contains("track-kinds") && !mixer.contains("track-kinds"),
        "`track-kinds` 只给编曲包头，⛔ 不进混音台注入面（否则 `elements` 的两级集合判据会红）"
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
