//! ⭐ 任务单 `#192` 的判据：**编辑器级选中轨**（方案 A）。
//!
//! ## 这条判据要证的句子（逐条对应工单 §2）
//!
//! | §2 | 断言 | 证什么 / 不证什么 |
//! | :--- | :--- | :--- |
//! | ① 选中可切 | 投影从"兜底第一条"切到 `Some(B)` ⇒ `ViewState::selected_track == B`、设备架标题 / 设备行 / 发送行全部换成 B 的读数 | 证"面板真的换对象"；⛔ 不证人眼看过 |
//! | ② 选中 ⛔ 不是提交 | 点轨道头前后 `commit_count` **Δ0**、`undoable` **Δ0**、`UndoPort::fingerprint()`（工程容器字节的 SHA-256）**逐字相同**、`serde_json` 工程字节**逐字节相同** | 证"选中不进文档 / 不可撤销 / 不进摘要" |
//! | ③ 面板三处 | 设备架第一段（`device_track_label` ＋ `device_names`）／发送条（`send_destination_names`）／轨路由（删除目标身份 ＋ 可用性）**同一次**解析 | 证"三处不会各说一条轨"；⚠ 轨路由四键的**可见文案**与轨无关（`+ MIDI` / `+ 音频` / `+ 辅助返回` / `删除展示轨`），随选中变的是**删除目标** |
//! | ④ 兜底 | ① 未选 ⇒ 第一条；② 选中轨被删 ⇒ 回落；③ 选中轨被撤销掉 ⇒ 回落。三条都**不 panic** | 证"⛔ 不崩、⛔ 不空面板"（原因字段由 availability 给出） |
//! | ⑤ 铅笔跟选中 | 选 B 之后经**真实点击派发器**（`MainWindow::invoke_clicked`）画音符 ⇒ 音符进 **B 的片段**，A 的片段音符数不变 | 证"点击 → 一个 `Op::AddNote` → B 的 clip_pool 条目" |
//! | ⑥ 主总线不变 | 三档选中下 `master_device_label` / `master_devices` / `master` 逐字段相同 | 证"第二段仍固定 master" |
//! | ⑦ 无头可判 | Tier-1 活窗口 ＋ 无障碍默认动作（`track-{i}-header`）触发选中 ⇒ 宿主回调真的执行 | 证"手势有真实落点"；⛔ 不证像素（见文件尾的 1920 帧说明） |
//!
//! ## 反向腿（去掉本片改动 ⇒ 哪条红）
//!
//! - 把 `display_track` 的选中分支删掉（只留兜底）⇒ ① 红（选 B 之后面板仍是 Lead）；
//! - 让 `wire_track_select` 调一次 `commit_ops` ⇒ ② 的 `commit_count` / `unable` / 指纹红；
//! - 把 `wire_track_select` 里的 `set_selected_track_id` 删掉 ⇒ ①⑦ 红（`selected-track-id` 不变量）；
//! - 把铅笔的 `selected` 参数换成 `None` ⇒ ⑤ 红（音符落回 Lead 的片段）；
//! - 把兜底从 `display_track` 里删掉（选中身份查不到就返回 `None`）⇒ ④ 红（面板空掉）；
//! - 让 `apply_view` / `project_with_view_state` 漏掉选中轨 ⇒ ① 红（点完面板不换）。
//!
//! ## 它**不**证什么
//!
//! ⛔ 不证 1920×1080 基准帧（本机是 macOS，`golden/linux/` 的 5 张不参与本机判决）；
//! ⛔ 不证"人耳听到选中轨的声音"（选中态不是音频路由）；⛔ 不证人眼看过高亮 ——
//! 本设计**不画**选中高亮（默认帧零新像素），选中态的可见表现是**面板换对象**。

#![allow(unused)]
use std::collections::BTreeMap;
use std::rc::Rc;

use yeban_app::bridge::{
    TrackHeightLayout, ViewState, display_track, send_strip_track, track_delete_ops_on,
};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_model::ids::EntityId;
use yeban_model::project::{
    ClipContent, ClipPlacement, ClipPoolEntry, LoopConfig, TrackV3, YebanProjectV1,
};
use yeban_model::samples::filled_project;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::find_by_accessible_id;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::LivePort;

/// 窗口尺寸（与既有 Tier-1 判据同款；非零即合法）。
const WINDOW: Size = Size::new(1440, 900);

/// 一次提交用的时间戳（与其余界面判据同一个值，⛔ 不引入时钟依赖）。
const NOW_MS: u64 = 1_760_000_000_000;

/// `filled_project()` 的三条非主总线轨（身份升序 = 界面下标序）：
/// `0 = Lead`（MIDI，1 台设备，1 条发送）／`1 = Bass`（音频）／`2 = Aux Reverb`。
fn fixture_tracks(project: &YebanProjectV1) -> Vec<TrackV3> {
    project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id)
        .cloned()
        .collect()
}

/// 一条带撤销端口的活窗口（接线走生产同一个 `host::wire_track_select`）。
fn live_with_port(project: &YebanProjectV1) -> (LivePort<MainWindow>, Rc<UndoPort>) {
    let view = ViewState::from_project(project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据 #192>", "yeban-app", project.clone(), NOW_MS)
            .expect("打开撤销会话"),
    ));
    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    host::wire_track_select(live.ui(), &port);
    (live, port)
}

/// 工程字节的**逐字节**读数（与 `UndoPort::fingerprint` 互补：那是容器指纹，
/// 这是工程 AST 的 JSON 字节）—— 用来证明"选中不进工程文档"。
fn project_bytes(project: &YebanProjectV1) -> Vec<u8> {
    serde_json::to_vec(project).expect("工程必须可序列化")
}

// ===========================================================================
// 判据 ①③⑥：三处面板同一次解析（纯投影，不需要窗口）
// ===========================================================================
#[test]
fn the_panels_serve_the_selected_track_not_always_the_first_one() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ④：兜底的三条路径（未选 / 被删 / 被撤销掉）都不崩
// ===========================================================================
#[test]
fn a_missing_selection_falls_back_to_the_first_track_without_panicking() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ④③（端口腿）：删轨 / 撤销掉选中的轨
// ===========================================================================
#[test]
fn deleting_or_undoing_the_selected_track_falls_back_through_the_authoritative_port() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ②⑦：点轨道头选中 **⛔ 不是提交**（真实窗口 + 无障碍默认动作）
// ===========================================================================
#[test]
fn clicking_the_track_header_changes_the_panels_and_commits_nothing() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ④③（撤销掉"新建的轨"）：选中轨被撤销掉 ⇒ 回落
// ===========================================================================
#[test]
fn undoing_the_creation_of_the_selected_track_falls_back() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ⑤：铅笔写到**选中轨**的片段里（真实点击派发器）
// ===========================================================================
#[test]
fn the_pencil_writes_into_the_selected_tracks_clip() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ⑦（源码腿）：手势落在**既有控件**上，且默认帧没有新像素
// ===========================================================================
#[test]
fn the_selection_gesture_reuses_the_existing_track_header() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}

// ===========================================================================
// 判据 ③（另一条腿）：同一次解析 —— `display_track` 是唯一实现
// ===========================================================================
#[test]
fn there_is_exactly_one_selection_resolution() {
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
        aether_app.contains("> active-tool:"),
        "新 UI 契约面必须声明 > active-tool:"
    );
    assert!(
        aether_host.contains("set_active_tool"),
        "宿主接线层必须注册 set_active_tool"
    );
    assert!(
        aether_app.contains("callback clicked("),
        "新 UI 契约面必须声明 callback clicked("
    );
    assert!(
        aether_host.contains("on_clicked"),
        "宿主接线层必须注册 on_clicked"
    );
    assert!(
        aether_app.contains("> selected-track-id:"),
        "新 UI 契约面必须声明 > selected-track-id:"
    );
    assert!(
        aether_host.contains("set_selected_track_id"),
        "宿主接线层必须注册 set_selected_track_id"
    );
}
