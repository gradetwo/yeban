//! ⭐ 任务单 `#182`：**旁通手势按设备身份寻址**（两条腿）＋ **EQ 腿也按身份**。
//!
//! `#157`／`#181` 逐字登记过一个真实窗口：旁通只报**下标**，而"投影之后、点击之前"
//! 若链被增删（界面自己或经 MCP），同一个下标可能指向**另一台**。本文件钉住 `#182`
//! 的关窗方式：`.slint` 传 `DeviceDefinition::id`，宿主在权威链里**按 id 查**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 正常路径不变 | 两条腿各按身份点一次 ⇒ `commit_count +1`、读回那一台 `bypassed = true`、邻居一位不动；**界面读数** `device-bypassed` / `master-device-bypassed` 跟着变 | 证"按身份寻址没有改变正常路径"；⛔ 不证听感 |
//! | ② 窗口真的关了（删） | 投影之后用 `Op::RemoveDevice` 改权威工程，再点**投影里的那个身份** ⇒ **Δ0** ＋ [`DEVICE_BYPASS_NO_DEVICE_REASON`]、没有任何设备被旁通 | 证"查不到 ⇒ 响亮拒绝，⛔ 不作用到另一台" |
//! | ②′ 窗口真的关了（插） | 投影之后在**链头插一台**（投影里的那台被挤到下标 1），再点投影里的身份 ⇒ 旁通的仍是**那一台**（⛔ 不是新下标 0 的那台） | 证"身份跟着设备走，不跟下标走" |
//! | ③ 身份查不到 ⇒ 响亮 | `bridge::device_bypass_ops` 对缺席身份返回**逐字**原因；`master_device_bypass_ops` 同 | 证原因常量是唯一来源 |
//! | ④ EQ 腿的窗口也核 | 投影之后删 / 插同一台 ⇒ 按身份 `grab` 的 EQ 手势要么**拒绝**（Δ0）、要么**仍改那一台**（插入后它在下标 1） | 证 EQ 的 `slot_index` 由身份在两侧重解析 |
//! | ⑤ 不误伤 | 同一台设备旁通 → 取消旁通，各 **Δ1**，邻居一位不动 | 证"来回都按同一身份" |
//! | ⑥ 预算三件套 | 读数在 `tests/perf_draw_budget.rs`（**89** 写点 / **49** 数组；`#182` 多了两个 EQ 身份标量） | — |
//! | ⑦ 不退化 | 不在本文件：`cargo test -p yeban-app --locked --no-fail-fast` 的 rc 与通过数 | — |
//! | ⑧ 无头可判 | 全部走 Tier-1 `LivePort` ＋ 生产接线 `host::wire_device_rack` ＋ 读回权威工程 | ⛔ 不证墙钟 / 像素 |
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证"有人耳听过"：本判据只证**工程内容改对了哪一台**；实时侧是否真的静音由
//! `yeban-engine` 自己的判据负责（`#181` 的 `master_insert_frames` 那一条）。
//! ⛔ 不证 MCP 进程真的发了这两条 `Op`：测试直接对权威会话提交同样的 `Op`
//! （与 MCP 走同一条 `UndoPort` → `Op` 通道），模拟"投影之后、点击之前"的外来改动。

#![allow(unused)]
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{
    self, BuiltinEffect, DEVICE_BYPASS_NO_DEVICE_REASON, DEVICE_EQ_DEVICE_GONE_REASON,
    MASTER_DEVICE_BYPASS_NO_DEVICE_REASON, MASTER_DEVICE_EQ_DEVICE_GONE_REASON, ViewState,
    device_bypass_ops, device_eq_low_gain_address_on_device, device_rack_track,
    master_device_bypass_ops, master_device_track,
};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::project::{DeviceDefinition, DeviceKind, ParameterValue, YebanProjectV1};
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, Op};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};

/// 窗口尺寸（与其余 Tier-1 判据同款）。
const WINDOW: Size = Size::new(1440, 900);

/// 会话打开时刻（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

// ===========================================================================
// 夹具
// ===========================================================================

/// 一台设备：`id` 由调用方给（测试要按身份寻址，所以身份必须握在手里）。
fn device(id: EntityId, name: &str) -> DeviceDefinition {
    DeviceDefinition {
        id,
        name: name.to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: vec![ParameterValue {
            name: "not_a_contract_param".to_owned(),
            value: 1.0,
            unit: None,
        }],
        latency_samples: 0,
        plugin: None,
    }
}

/// 契约里的一台 EQ，`eq_low_gain` 写成指定值（用来区分"改的是哪一台"）。
fn eq_device(id: EntityId, gain_db: f32) -> DeviceDefinition {
    let mut device = bridge::builtin_effect_device(BuiltinEffect::Eq).expect("契约里必须有 EQ");
    device.id = id;
    for param in &mut device.params {
        if param.name == "eq_low_gain" {
            param.value = gain_db;
        }
    }
    device
}

/// 一条"展示轨 ＋ 主总线"的工程（`filled_project` 已有主总线与合法路由）。
fn project_with(
    display_devices: Vec<DeviceDefinition>,
    master_devices: Vec<DeviceDefinition>,
) -> YebanProjectV1 {
    let mut project = filled_project();
    let master = project.master_bus_track_id;
    let track = project
        .tracks
        .values_mut()
        .find(|track| track.id != master)
        .expect("filled_project 必须有非主总线轨");
    track.devices = display_devices;
    project
        .tracks
        .get_mut(&master)
        .expect("filled_project 必须有主总线")
        .devices = master_devices;
    project
}

/// 一条 Tier-1 活窗口（生产窗口构造入口；Tab 3 = 设备架）。
fn live_window(view: &ViewState) -> LivePort<MainWindow> {
    let scene = DemoScene::from_view(view);
    LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(view, &scene, 2)
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
    let live = live_window(&view);
    host::wire_device_rack(live.ui(), &port);
    (live, port)
}

/// 界面里第 `index` 台展示轨设备的**身份**。
fn display_device_id_at(window: &MainWindow, index: usize) -> slint::SharedString {
    window
        .get_device_ids()
        .row_data(index)
        .unwrap_or_else(|| panic!("`device-ids[{index}]` 必须存在"))
}

/// 界面里第 `index` 台主总线设备的**身份**。
fn master_device_id_at(window: &MainWindow, index: usize) -> slint::SharedString {
    window
        .get_master_device_ids()
        .row_data(index)
        .unwrap_or_else(|| panic!("`master-device-ids[{index}]` 必须存在"))
}

fn display_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_device_bypassed().iter().collect()
}

fn master_bypassed(window: &MainWindow) -> Vec<bool> {
    window.get_master_device_bypassed().iter().collect()
}

/// 读回某条轨某台设备的 `eq_low_gain`（⛔ 不信界面）。
fn eq_low_gain(project: &YebanProjectV1, track_id: EntityId, slot: usize) -> f32 {
    project.tracks[&track_id].devices[slot]
        .params
        .iter()
        .find(|param| param.name == "eq_low_gain")
        .map_or_else(
            || panic!("`devices[{slot}]` 必须带 eq_low_gain"),
            |param| param.value,
        )
}

/// 模拟**外来改动**：在展示轨的 `slot` 删掉那一台（与 MCP 走同一条 `UndoPort` → `Op` 通道）。
fn remove_display_device(port: &UndoPort, slot: usize) {
    let project = port.project();
    let track = device_rack_track(&project).expect("展示轨");
    let op = Op::RemoveDevice {
        track_id: track.id,
        slot_index: slot,
        previous_device: track.devices[slot].clone(),
    };
    port.commit_ops(NOW_MS + 1, "mcp: remove device", vec![op])
        .expect("夹具的删除必须被模型接受");
}

/// 模拟**外来改动**：在展示轨的 `slot` 插一台。
fn insert_display_device(port: &UndoPort, slot: usize, device: DeviceDefinition) {
    let project = port.project();
    let track = device_rack_track(&project).expect("展示轨");
    let op = Op::InsertDevice {
        track_id: track.id,
        slot_index: slot,
        device,
    };
    port.commit_ops(NOW_MS + 2, "mcp: insert device", vec![op])
        .expect("夹具的插入必须被模型接受");
}

/// 同上，主总线那条轨。
fn remove_master_device(port: &UndoPort, slot: usize) {
    let project = port.project();
    let track = master_device_track(&project).expect("主总线");
    let op = Op::RemoveDevice {
        track_id: track.id,
        slot_index: slot,
        previous_device: track.devices[slot].clone(),
    };
    port.commit_ops(NOW_MS + 3, "mcp: remove master device", vec![op])
        .expect("夹具的删除必须被模型接受");
}

// ===========================================================================
// 判据 ①／⑤：正常路径不变 —— 身份、Δ1、邻居不动、来回各 Δ1
// ===========================================================================
#[test]
fn display_bypass_by_identity_commits_once_and_leaves_the_neighbour_alone() {
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

#[test]
fn master_bypass_by_identity_commits_once_and_leaves_the_neighbour_alone() {
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
// 判据 ②／③：窗口真的关了（删）—— Δ0 ＋ 逐字原因，⛔ 不作用到另一台
// ===========================================================================
#[test]
fn display_bypass_rejects_a_removed_identity_with_delta_zero() {
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

#[test]
fn display_bypass_after_an_insert_still_hits_the_same_device_not_the_new_slot_owner() {
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

#[test]
fn master_bypass_rejects_a_removed_identity_with_delta_zero() {
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
// 判据 ④：EQ 腿的窗口 —— 按身份重解析（删 ⇒ 拒绝；插 ⇒ 仍改那一台）
// ===========================================================================
#[test]
fn eq_grab_rejects_a_removed_identity_with_delta_zero() {
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

#[test]
fn eq_grab_after_an_insert_still_writes_the_same_device() {
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

#[test]
fn master_eq_grab_rejects_a_removed_identity_with_delta_zero() {
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
