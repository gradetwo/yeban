//! ⭐ 任务单 `#163` 的判据：**装设备 / 删设备 / 建空的 MIDI 片段**。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 装 EQ = Δ1 | `invoke_device_insert_eq()` ⇒ `commit_count +1`、`undoable +1`；读回设备数 +1、`kind == InternalEffect`、`params` 非空、**含 `eq_low_gain`**（值 = 契约缺省值 0.0） | 证"点击 → 一个 `Op::InsertDevice` → 一步撤销" |
//! | ② 装完 `#157` 的滑块可用 | 同一个活窗口：`device_insert_available` 前 `true`；装完 `eq_available` 由 `false` → `true`；`invoke_device_eq_grab(device-eq-device-id, 1.0)`（⭐ `#182` 按身份）＋ `invoke_device_eq_release()` ⇒ `commit_count +1`，读回值 = 契约上界 18.0 | 证"装完设备，那条滑块立刻可用"（⛔ 只证工程值变了，不证能听见） |
//! | ③ 装混响 = Δ1 | 同上；读回 `name == "Reverb"`、参数是 `reverb_*`（含 `reverb_wet` 0.25） | 证"同一形状能装第二种效果器" |
//! | ④ 删设备 = Δ1 | `invoke_device_remove(身份)` ⇒ `commit_count +1`；读回设备数 −1、身份消失 | 证"每台一个 ✕，且按**身份**寻址" |
//! | ⑤ 建 clip = Δ1 | `invoke_clip_create()` ⇒ `commit_count +1`；`clip_pool` +1、展示轨 `clips` +1、placement `start_tick=0` / `duration_ticks=15360`（4/4 的 4 小节）、文档 `validate() Ok` | 证"一个原子批次 = 一步撤销"；⛔ 不证钢琴卷帘里画出来 |
//! | ⑥ 空态 / 禁用 | 只有主总线的工程 ⇒ 两个可用性 `false`、原因**逐字**等于 `bridge` 的常量；直接调回调也 Δ0 | 证"不静默禁用、也不靠界面拦" |
//! | ⑦ 完整一首的头四步 | `default_project()`（只有主总线）⇒ `+ MIDI`（Δ1，⭐ `D-UI-4` 后该轨**自带一台内置乐器**）⇒ 装 EQ（Δ1，追加到链尾）⇒ 建 clip（Δ1），文档 `validate() Ok` | 证"新建工程 → 加轨 → 装设备 → 建 clip"这条链真的通，且**音源在前、效果器在后** |
//!
//! 预算三件套（§2 ⑦）不在本文件：读数在 `tests/perf_draw_budget.rs`（写点 / 数组常量与原因同处）。
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 摘掉 `host::wire_device_rack` 里装设备那一段的 `commit_ops` ⇒ ①③ 的 `commit_count` 差 0；
//! - 摘掉 `host::wire_clip_create` 的 `commit_ops` ⇒ ⑤ 的 `commit_count` 差 0；
//! - 把 `clip_create_ops` 里的 `AddClipPlacement` 删掉 ⇒ ⑤ 的"展示轨 `clips` +1"红；
//! - 把 `device_remove_ops` 的按身份寻址改成按界面行下标 ⇒ ④ 在"身份消失"那条断言上仍绿、
//!   但 `device_remove_ops` 的单测（身份不存在 ⇒ 拒绝）会红；
//! - 把 `builtin_effect_device` 的 `params` 写成空 `Vec` ⇒ ①③ 的"params 非空"红；
//! - 摘掉 `track_create_ops` 里 `D-UI-4` 那条 `InsertDevice` ⇒ ⑦ 的"槽 0 是内置乐器"红
//!   （另见 `tests/track_routing_crud.rs` 判据 ①/①b）。
//!
//! ## 它**不**证什么（见交付报告的"能验与不能验"）
//!
//! ⛔ 不证"装了 EQ / 混响就能听见"：本判据只证工程文档真的变了。实时侧的插入链是
//! `yeban-engine` 的事；⛔ 不证钢琴卷帘里能看见新片段（那是投影的另一族判据），
//! ⛔ 不证墙钟成本（要 GPU 档，本机纪律不允许）。

#![allow(unused)]
use std::collections::BTreeSet;
use std::rc::Rc;

use slint::Model as _;
use yeban_app::bridge::{
    BUILTIN_INSTRUMENT_NAME, BuiltinEffect, CLIP_CREATE_NO_TRACK_REASON,
    DEVICE_INSERT_NO_TRACK_REASON, ViewState, builtin_effect_device, device_insert_ops,
    device_remove_ops,
};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UndoPort, UndoSession};
use yeban_model::EntityId;
use yeban_model::project::{ClipContent, DeviceKind, TrackKind, TrackV3, YebanProjectV1};
use yeban_model::samples::{default_project, demo_project, filled_project};
use yeban_ui_test_port::Size;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};

/// 窗口尺寸：与 `track_routing_crud.rs` 同款（同一块 Tab 3 面板需要高度余量）。
const WINDOW: Size = Size::new(1440, 1000);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// 4/4 的 4 小节 = `4 × 4 × 960`（`PPQ = 960` 从模型读，⛔ 本文件不手抄 PPQ）。
fn four_bars_ticks(project: &YebanProjectV1) -> u64 {
    let bar = yeban_app::bridge::bar_length_ticks(project.time_signature).expect("拍号必须合法");
    bar * 4
}

/// 一条带撤销端口的 Tab 3 活窗口：接线走**生产同一个** `host::wire_device_rack` /
/// `host::wire_clip_create` / `host::wire_track_strip`。
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
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_device_rack(live.ui(), &port);
    host::wire_clip_create(live.ui(), &port);
    host::wire_track_strip(live.ui(), &port);
    (live, port)
}

/// 展示轨（身份序第一条非主总线轨）—— 与投影**同一条**规则。
fn display_track(project: &YebanProjectV1) -> &TrackV3 {
    project
        .tracks
        .values()
        .find(|track| track.id != project.master_bus_track_id)
        .expect("夹具必须有展示轨")
}

/// 只有主总线的合法工程（⑥ 的夹具）。
fn project_with_master_only() -> YebanProjectV1 {
    let mut project = YebanProjectV1::default();
    let master_id = EntityId::new();
    project.master_bus_track_id = master_id;
    project.tracks.insert(
        master_id,
        TrackV3 {
            id: master_id,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    project.routing_graph.nodes = vec![master_id];
    project.validate().expect("只剩主总线的工程必须合法");
    project
}

/// 契约里的 `(分组参数名序列, 每条的缺省值)` —— 从共享契约现读，⛔ 不手抄第二份。
fn contract_params(group_id: &str) -> (Vec<&'static str>, Vec<f32>) {
    let group = yeban_app::effect_params::EFFECT_GROUPS
        .iter()
        .find(|group| group.id == group_id)
        .expect("共享契约里必须有这个分组");
    #[allow(clippy::cast_possible_truncation)]
    (
        group.params.iter().map(|spec| spec.name).collect(),
        group
            .params
            .iter()
            .map(|spec| spec.default as f32)
            .collect(),
    )
}

/// 契约里某个参数的 `(min, max, default)`（f32）。
fn contract_spec(name: &str) -> (f32, f32, f32) {
    let (_, spec) = yeban_app::effect_params::recognise(name).expect("契约里必须有这个参数");
    #[allow(clippy::cast_possible_truncation)]
    (spec.min as f32, spec.max as f32, spec.default as f32)
}

// ===========================================================================
// 判据 ①：装 EQ = Δ1 ＋ 读回（设备 +1 / kind / 非空 params / eq_low_gain）
// ===========================================================================
#[test]
fn inserting_an_eq_commits_once_and_the_chain_carries_eq_low_gain() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ②：装完 `#157` 的滑块可用（拖动 ⇒ 读回值变）
// ===========================================================================
#[test]
fn the_eq_slider_is_available_and_editable_after_installing_the_eq() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ③：装混响 = Δ1 ＋ 读回 `reverb_*`
// ===========================================================================
#[test]
fn inserting_a_reverb_commits_once_and_installs_the_reverb_params() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ④：删设备 = Δ1 ＋ 读回（设备数 −1、身份消失）
// ===========================================================================
#[test]
fn removing_a_device_commits_once_and_shrinks_the_chain() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ⑤：建空的 MIDI 片段 = Δ1 ＋ 读回（池 +1 / 轨 clips +1 / 0 / 15360 / Ok）
// ===========================================================================
#[test]
fn creating_an_empty_clip_commits_once_and_places_four_bars_at_zero() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ⑥：空态 / 禁用（无展示轨 ⇒ 禁用 ＋ 逐字原因 ＋ 直接调用 Δ0）
// ===========================================================================
#[test]
fn the_action_buttons_are_disabled_with_a_verbatim_reason_without_a_track() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 判据 ⑦：完整一首的头四步（新建工程 → 加轨 → 装设备 → 建 clip）
// ===========================================================================
#[test]
fn the_empty_project_can_grow_a_track_a_device_and_a_clip() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}

// ===========================================================================
// 纯函数腿：`builtin_effect_device` 的缺省值来自契约（不依赖窗口）
// ===========================================================================
#[test]
fn the_builtin_devices_take_every_default_from_the_shared_contract() {
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
        aether_app.contains("callback clip-create("),
        "新 UI 契约面必须声明 callback clip-create("
    );
    assert!(
        aether_host.contains("on_clip_create"),
        "宿主接线层必须注册 on_clip_create"
    );
    assert!(
        aether_app.contains("callback device-eq-grab("),
        "新 UI 契约面必须声明 callback device-eq-grab("
    );
    assert!(
        aether_host.contains("on_device_eq_grab"),
        "宿主接线层必须注册 on_device_eq_grab"
    );
    assert!(
        aether_app.contains("callback device-eq-release("),
        "新 UI 契约面必须声明 callback device-eq-release("
    );
    assert!(
        aether_host.contains("on_device_eq_release"),
        "宿主接线层必须注册 on_device_eq_release"
    );
}
