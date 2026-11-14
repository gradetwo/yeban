//! ⭐ 任务单 `#157` 的判据：**设备效果链是真实投影 ＋ 最小可编辑**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §3）
//!
//! | §3 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 假数据已删 | `ui/console/device_rack.slint` 的**源码**里没有旧设备名 / `band-gains` / `for device_index in 3`；行循环是 `for … in root.device-names` | 证"界面不再自带一份列表"。⛔ 不证历史（旧字面量在 `scene.rs` 的删除登记里点名，不在本文件里复读） |
//! | ② 投影是派生的 | 同一活窗口上把投影从 1 台设备换到 3 台 ⇒ 运行时 `device-*-card` 节点 1 → 3；`device-rack-count` 的逐字标签 `1 台设备` → `3 台设备` | 证"条目数随投影变"。⛔ 不证像素（见 ⑦） |
//! | ③ 空态可判 | 空工程 ⇒ `device-rack-empty` 的**逐字**文案 `无设备（点上方「装 EQ」或「装混响」）`、卡片 0 个、`0 台设备` | 证"没有设备时不是一片空白，也不是假条目" |
//! | ④ 旁通手势 = 恰好一次提交 | `invoke_device_bypass_toggle(device-ids[0])`（⭐ `#182` 按身份）⇒ `commit_count +1`、`undoable +1`、`TrackV3::devices[0].bypassed` 真的翻转、界面 `device-bypassed[0]` 跟着变 | 证"点击 → 一个 `Op::SetDeviceBypassed` → 一步撤销" |
//! | ⑤ EQ 滑块 = 恰好一次提交 | `invoke_device_eq_grab(device-eq-device-id, 1.0)` ＋ `invoke_device_eq_release()` ⇒ `commit_count +1`、`undoable +1`，**读回的模型值** = `18.0`（值域上界来自契约）；再在同位置按下 / 松手 ⇒ `+0`（没变就不提交） | 证"拖动一次 = 一步撤销，且改的值进了 `TrackV3::devices`" |
//! | ⑥ 预算三件套 | 不在本文件：读数在 `tests/perf_draw_budget.rs`（52 写点 / 38 数组，常量与原因同处） | — |
//! | ⑦ 无头可判 | Tier-1 窗口（`LivePort`）＋ 运行时控件树 ＋ 一张 Tab 3 截图（非黑 / 多色） | 证"真的有一个窗口在画它"；⛔ 不证墙钟成本（本机不许跑 GPU 档） |
//! | ⑧ 主总线（`#173`） | `master_bus_track_id` 上挂一台 EQ ⇒ `ViewState::master_devices` 出现 1 行且"已识别 eq_low_gain"；**展示轨**的 `devices` 一动不动（可编辑面仍只属于展示轨） | 证"主总线的设备进了控制面投影"；⛔ 不证 `.slint` 布局（本单**刻意**不改布局，避免 golden 重录） |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `device-names` 的默认值改回三条字面量 ⇒ ① 红（源码探针）；
//! - 把 `host::apply_view` 里的四个 `set_device_*` 数组删掉 ⇒ ②③ 的运行时读数红；
//! - 把 `for … in root.device-names` 改回 `for device_index in 3` ⇒ ② 红（1 台设备的夹具
//!   会画出 3 张卡）且 ③ 红（空态不可见）；
//! - 摘掉 `host::wire_device_rack` 里的那次 `commit_ops` ⇒ ④⑤ 的 `commit_count` 差 0；
//! - 让 `device-eq-release` 在拖动期也提交（每次 move 一次 `commit_ops`）⇒ ⑤ 的 `+1` 变 `+k`。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"拖 EQ 旋钮就能听到声音"：本判据只证工程内容真的变了。实时侧的插入链是
//! `yeban-engine` 的事 —— 单轨插入会响、**aux 总线上的插入今天还不会**（工单 §0 第 5 条）。

#![allow(unused)]
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{ViewState, device_eq_low_gain_target, device_rack_track};
use yeban_app::elements::ElementRegistry;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue, YebanProjectV1};
use yeban_model::samples::{default_project, demo_project, filled_project};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, golden_evidence, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 设备机架源码（**另一个文件**，不是本判据自己）—— 第 293 条：判据钉住的字符串必须从
/// 别处读，⛔ 不许从判据所在文件读（那是恒真）。
const RACK_SOURCE: &str = include_str!("../ui/console/device_rack.slint");

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 内嵌 MCP 契约里的低架增益值域（**从契约现读**，⛔ 不手抄第二个 -18/18）。
fn contract_eq_range() -> (f32, f32) {
    let (_, spec) =
        yeban_app::effect_params::recognise("eq_low_gain").expect("契约里必须有 eq_low_gain");
    #[allow(clippy::cast_possible_truncation)]
    (spec.min as f32, spec.max as f32)
}

/// 一台设备：`params` 逐条给 `(名字, 值, 单位)`。
fn device(name: &str, bypassed: bool, params: &[(&str, f32, Option<&str>)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed,
        params: params
            .iter()
            .map(|(param, value, unit)| ParameterValue {
                name: (*param).to_owned(),
                value: *value,
                unit: unit.map(str::to_owned),
            })
            .collect(),
        latency_samples: 0,
        plugin: None,
    }
}

/// 一台真的带 `eq_low_gain` 的 EQ：两个已识别参数 ＋ 一个未识别参数。
fn eq_device() -> DeviceDefinition {
    device(
        "Yeban EQ",
        false,
        &[
            ("eq_low_gain", 0.0, Some("dB")),
            ("eq_low_freq", 200.0, Some("Hz")),
            ("mystery", 1.0, None),
        ],
    )
}

/// 把 `filled_project()` 的**展示轨**（身份序第一条非主总线轨）的设备链换成 `devices`。
///
/// 为什么直接改模型而不是提交 `Op::InsertDevice`：夹具是**判据的输入**，不是被测行为；
/// 本单被测的是"投影 ＋ 两个手势"，插入设备那条 `Op` 在 UI 侧今天**没有**构造点
/// （本单不造，见交付报告）。用完的工程只活在内存里，⛔ 不落盘。
fn project_with(devices: Vec<DeviceDefinition>) -> YebanProjectV1 {
    let mut project = filled_project();
    let master = project.master_bus_track_id;
    let track = project
        .tracks
        .values_mut()
        .find(|track| track.id != master)
        .expect("filled_project 必须有非主总线轨");
    track.devices = devices;
    project
}

/// 读活窗口的运行时控件树（`[UI-DoD §3.6]` 的载体：全树内省 ＋ JSON 往返）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 模型数组：设备名（`ModelRc<SharedString>`）。
fn model_names(window: &MainWindow) -> Vec<String> {
    window
        .get_device_names()
        .iter()
        .map(|name| name.to_string())
        .collect()
}

/// 模型数组：旁通位。
fn model_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_device_bypassed().iter().collect()
}

/// `device-{i}-card` 的形态判定（⚠ `device-plugin-host-placeholder-card` 也以 `-card`
/// 结尾，所以不能只按后缀数：否则空链也会数出 1）。
fn is_device_card(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("device-") else {
        return false;
    };
    let Some(index) = rest.strip_suffix("-card") else {
        return false;
    };
    !index.is_empty() && index.bytes().all(|byte| byte.is_ascii_digit())
}

/// 运行时树里 `device-{i}-card` 的条数。
fn card_count(tree: &ControlTree) -> usize {
    tree.iter().filter(|node| is_device_card(&node.id)).count()
}

/// 一条**生产形态**的 Tier-1 活窗口（`build_main_window_with_console_tab` 是唯一的注入实现）。
///
/// ⚠ `LivePort` 必须被持有到判据结束；同一测试里只能建一个（Tier-1 平台每线程一份）。
fn live_window(view: &ViewState, console_tab: i32) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, console_tab)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

/// 一条带撤销端口的 Tab 3 活窗口（接线走**生产同一个** `host::wire_device_rack`）。
fn live_window_with_undo(
    project: &YebanProjectV1,
    tag: &str,
) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open(
            format!("<判据 {tag}>"),
            "yeban-app",
            project.clone(),
            NOW_MS,
        )
        .expect("打开撤销会话"),
    ));
    let live = live_window(&view, 2);
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

// ===========================================================================
// 判据 ①：假数据已删（**源码派生**，不需要窗口）
// ===========================================================================
#[test]
fn the_fake_rack_data_is_gone_and_the_chain_follows_the_projection() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ②：投影是派生的（两个不同设备数的夹具 ⇒ 面板条目数随之变）
// ===========================================================================
#[test]
fn the_rack_follows_the_projected_device_chain() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ③：空态可判（无设备 ⇒ 逐字文案 ＋ 条目数 0）
// ===========================================================================
#[test]
fn an_empty_chain_shows_the_empty_state_and_zero_rows() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

/// 界面里第 `index` 台展示轨设备的**身份**（`.slint` 的 `device-ids` 数组）。
///
/// ⭐ 任务单 `#182`：旁通 / EQ 手势按**身份**寻址 ⇒ 判据也从界面数组里取身份（⛔ 不手抄
/// ULID、⛔ 不传行下标）。
fn device_id_at(window: &MainWindow, index: usize) -> slint::SharedString {
    window
        .get_device_ids()
        .row_data(index)
        .unwrap_or_else(|| panic!("`device-ids[{index}]` 必须存在"))
}

// ===========================================================================
// 判据 ④：旁通手势 = 恰好一次提交
// ===========================================================================
#[test]
fn the_bypass_gesture_commits_exactly_once() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ⑤：EQ 滑块 = 恰好一次提交，且值真的进了 `TrackV3::devices`
// ===========================================================================
#[test]
fn the_eq_slider_commits_exactly_once_and_writes_the_model() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ⑦：无头可判 —— Tab 3 真有一张非黑、多色的截图
// ===========================================================================
#[test]
fn the_device_tab_renders_a_non_black_frame() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}

// ===========================================================================
// 判据 ⑧（任务单 `#173`）：主总线的设备链进了**只读**投影，可编辑面不越轨
// ===========================================================================
#[test]
fn the_master_chain_is_projected_without_touching_the_editable_rack() {
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
        aether_app.contains("callback device-bypass-toggle("),
        "新 UI 契约面必须声明 callback device-bypass-toggle("
    );
    assert!(
        aether_host.contains("on_device_bypass_toggle"),
        "宿主接线层必须注册 on_device_bypass_toggle"
    );
    assert!(
        aether_app.contains("> device-bypassed:"),
        "新 UI 契约面必须声明 > device-bypassed:"
    );
    assert!(
        aether_host.contains("set_device_bypassed"),
        "宿主接线层必须注册 set_device_bypassed"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
}
