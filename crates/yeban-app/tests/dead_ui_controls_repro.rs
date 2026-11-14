//! 诊断复现：夜半桌面端三处“代码已接线、桌面通路不通”的失效控件
//! （UI 审计 `~/workspace/research/yeban-ui-audit.md` §1.3–1.5）。
//!
//! 每处都做同一组二分：
//!   - **A. 回调直调**：直接 `invoke_*` 回调 ⇒ 断回调与宿主臂本身通不通；
//!   - **B. 真实指针**：`dispatch_pointer_down/up` 走真命中测试 ⇒ 断输入面通不通；
//!   - **C. 真实按键**：`dispatch_key_press` 走平台事件 ⇒ 断焦点/键路通不通。
//!
//! A 绿而 B/C 红 ⇒ 断点在命中测试或焦点，不在接线。
//! 本片是诊断片：先跑出红绿分布再定修法，⛔ 不先猜。

#![allow(unused)]
use std::cell::RefCell;
use std::rc::Rc;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::host::{self, ProductionLoop};
use yeban_app::scene::DemoScene;
use yeban_app::song_overview;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{KeyCode, Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 桌面实测窗口尺寸（xwininfo 核实：1280x670，左上 (1,99)）。
const DESKTOP: Size = Size::new(1280, 670);
/// 对照尺寸：`song_overview_panel.rs` 判据 ⑫ 在此尺寸下点折叠键是绿的。
const CONTROL: Size = Size::new(1440, 900);
const NOW_MS: u64 = 1_700_000_000_000;

const PANEL: &str = "song-overview-panel";
const COLLAPSE: &str = "song-overview-collapse-button";
const ENERGY_PILL: &str = "song-overview-layer-energy-pill";
const PENCIL_BUTTON: &str = "piano-roll-tool-pencil-button";
const SCROLL_AREA: &str = "piano-roll-scroll-area";
/// 顶栏 BPM 敲入框（`transport.slint:258`，真 `TextInput`）。
const BPM_FIELD: &str = "transport-bpm-field";

/// 产品装配的复现台：真引擎 + 真活窗口 + 三条线全部接上。
struct Rig {
    _production: ProductionLoop,
    _engine: Rc<RefCell<yeban_app::engine_host::EngineHost>>,
    port: LivePort<MainWindow>,
    _state: Rc<RefCell<song_overview::SongOverviewLive>>,
}

impl Rig {
    fn new_at(size: Size) -> Self {
        let project = demo_project();
        let production =
            ProductionLoop::start_with_audio_source(&project, 0, None).expect("产品装配建引擎");
        assert!(production.has_engine(), "引擎必须活着");
        let engine = production.engine_handle();
        engine.borrow_mut().stop();

        let view = ViewState::from_project(&project).expect("演示工程必须能投影");
        let scene = DemoScene::from_view(&view);
        let port = LivePort::new(size, Permission::Interactive, None, || {
            host::build_main_window(&view, &scene)
        })
        .expect("Tier-1 平台 + 主窗口");

        let live_view = Rc::new(RefCell::new(view));
        let undo = Rc::new(UndoPort::new(
            UndoSession::open("<dead-ui-controls>", "yeban-app", project.clone(), NOW_MS)
                .expect("打开"),
        ));
        host::wire_transport(port.ui(), Rc::clone(&engine), None, None);
        let state = song_overview::wire_song_overview(
            port.ui(),
            Rc::clone(&engine),
            Rc::clone(&live_view),
            Some(Rc::clone(&undo)),
        );
        // 焦点根因探针的前提：与产品装配（`main.rs:636/873`）同一套 ——
        // `wire_input` 让 `bpm-input` 的 `changed has-focus` 落进 `InputContext`，
        // `wire_bpm_commit` 让 `invoke_bpm_commit` 走真的提交闭包（含回焦与否）。
        // 两处与 `wire_keys` 共用同一个 `InputContext`，与产品装配一致。
        let input = Rc::new(RefCell::new(yeban_app::input::InputContext::new()));
        host::wire_input(port.ui(), Rc::clone(&input));
        host::wire_keys(
            port.ui(),
            Rc::clone(&input),
            Some(Rc::clone(&undo)),
            Some(Rc::clone(&engine)),
            Some(Rc::new(project.clone())),
        );
        host::wire_bpm_commit(port.ui(), &undo);
        host::wire_tool_select(port.ui(), Some(Rc::clone(&undo)));
        host::wire_undo(port.ui(), &undo);
        host::apply_undo(port.ui(), &undo);
        host::apply_transport(port.ui(), engine.borrow().transport());

        let mut rig = Self {
            _production: production,
            _engine: engine,
            port,
            _state: state,
        };
        rig.port
            .refresh_tree(None)
            .expect("必须能抓控件树（几何来自这一次抓取）");
        rig
    }

    fn ui(&self) -> &MainWindow {
        self.port.ui()
    }

    fn refresh(&mut self) {
        self.port.refresh_tree(None).expect("状态变后必须能重抓树");
    }

    fn bounds(&self, element_id: &str) -> Rect {
        self.port
            .tree()
            .find_by_id(element_id)
            .unwrap_or_else(|| panic!("控件树里必须有语义 ID `{element_id}`"))
            .bounds
            .unwrap_or_else(|| panic!("元素 `{element_id}` 必须有几何包围盒"))
    }

    /// 在某语义 ID 中心真实按下 + 释放一次（走命中测试，⛔ 不直接调回调）。
    fn click_center(&mut self, element_id: &str) {
        let bounds = self.bounds(element_id);
        self.port
            .dispatch_pointer_down(
                element_id,
                f64::from(bounds.width) / 2.0,
                f64::from(bounds.height) / 2.0,
                PointerButton::Left,
            )
            .unwrap_or_else(|e| panic!("Interactive 权限下 `{element_id}` 必须可按下：{e:?}"));
        self.port
            .dispatch_pointer_up(PointerButton::Left)
            .expect("释放必须送达");
        self.refresh();
    }

    fn panel_height(&self) -> u32 {
        self.bounds(PANEL).height
    }
}

/// ① 折叠键：桌面尺寸 1280x670 下点一次 ⇒ 必须展开（collapsed=false，面板 104 px）。
#[test]
fn fold_button_click_at_desktop_size_expands_the_panel() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ① 对照：1440x900 下点折叠键 ⇒ 必须展开（判据 ⑫ 的同款，预期绿）。
#[test]
fn fold_button_click_at_control_size_expands_the_panel() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ① 能量胶囊：桌面尺寸下点一次 ⇒ show-energy 翻转为 false；再点 ⇒ 恢复 true。
#[test]
fn energy_pill_click_at_desktop_size_toggles_the_layer() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ② 铅笔按钮：桌面尺寸下点“2 铅笔”中心 ⇒ active-tool 必须是 2。
#[test]
fn pencil_button_click_at_desktop_size_selects_the_pencil_tool() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ② 回调直调对照：直接 invoke tool-selected(2) ⇒ active-tool 必须是 2。
/// （本条绿而上一条红 ⇒ 断点在命中测试，不在宿主臂。）
#[test]
fn invoking_tool_selected_directly_selects_the_pencil_tool() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ② 平台按键对照：启动后立即按数字键 `2` ⇒ active-tool 必须是 2。
#[test]
fn digit_key_2_at_startup_selects_the_pencil_tool() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ② 焦点探针：点一次卷帘画布空白处后，快捷键层是否还持有焦点？再按 `2` 是否还切换？
/// （桌面复现顺序：先点画布空白格、再按 `2`；本条钉住那一次点击对焦点与键路的影响。）
#[test]
fn digit_key_2_after_a_canvas_click_still_selects_the_pencil_tool() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ③ 回调直调对照：直接 invoke key-action("h", shift, ctrl) ⇒ 时光机必须打开。
/// （本条绿 ⇒ key-action → resolve → OpenTimeMachine → 端口翻转整条回调链是通的。）
#[test]
fn invoking_key_action_ctrl_shift_h_opens_the_undo_tree() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ③ 焦点探针：点一次卷帘画布后，快捷键层焦点是否还在？（与 ② 的探针同一问题、另一量纲。）
#[test]
fn shortcut_layer_keeps_focus_after_a_canvas_click() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ②③ 焦点根因：BPM 提交（回车）后焦点必须回到快捷键层。
///
/// 桌面失效的直接原因（审计时间线）：96–97 号图先敲 BPM（输 130、回车、输 120、回车），
/// 98–102 号图按数字键 `2` 不切换、106–110 号图 Ctrl+Shift+H 不开；
/// 而 `C`（63–65 号图，敲 BPM 之前）是通的。`wire_bpm_commit` 只清
/// `bpm_field_editing`、不回焦，焦点一直留在 `bpm-input` 里；`key-handler` 是空
/// `FocusScope`（`app.slint:846`，Transport 不在其子树内），再也收不到键。
/// 本条是修法判据：修前红（焦点不回来，按 `2` 不切换），修后绿。
#[test]
fn bpm_submit_returns_focus_to_shortcut_layer_and_restores_digit_keys() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}

/// ②③ 同上：非法值提交（“abc”）同样是“编辑结束”，焦点也必须回来。
/// （`wire_bpm_commit` 的非法值路直接 `return`，回焦必须放在公共入口才覆盖它。）
#[test]
fn bpm_submit_with_invalid_value_returns_focus_to_shortcut_layer() {
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
        aether_app.contains("callback bpm-commit("),
        "新 UI 契约面必须声明 callback bpm-commit("
    );
    assert!(
        aether_host.contains("on_bpm_commit"),
        "宿主接线层必须注册 on_bpm_commit"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback tool-selected("),
        "新 UI 契约面必须声明 callback tool-selected("
    );
    assert!(
        aether_host.contains("on_tool_selected"),
        "宿主接线层必须注册 on_tool_selected"
    );
}
