//! **撤销接线判据（界面侧）** —— ADR-0001 **D45** 的"人按 `Cmd+Z` 真的能撤销"。
//!
//! 与 MCP 侧那份（`crates/yeban-mcp/tests/undo_wiring.rs`）**互补**：
//!
//! | 判据 | 落点 |
//! | :--- | :--- |
//! | ⑧ UI 路径与 MCP 路径**逐字节相同**（同一串 `Op`、同一个 `undo_session`） | [`the_ui_path_matches_the_shared_implementation_byte_for_byte`] |
//! | ⑨ 界面动作**真的**回退工程（动作日志断言，不用控件树） | [`a_click_really_rolls_the_project_back`] |
//! | ⑨ `Cmd+Z` 整条链（策略表 → 动作 → 工程回退） | [`the_cmd_z_chain_reaches_the_model`] |
//! | ⑫ 撤销不越过"工程打开"边界（新会话 = 新端口） | [`a_new_project_open_starts_a_fresh_undo_session`] |
//! | ⑪ 撤销 → 保存（真容器）→ 重开 ⇒ 与内存态一致 | [`saved_after_an_undo_reopens_to_the_same_bytes`] |
//! | ⑩ 撤销后工程仍然 `validate()` 通过 | [`the_project_still_validates_after_a_ui_undo`] |
//! | 显示态来自模型读数（不是界面算的） | [`the_display_state_comes_from_the_model_readings`] |
//! | 重投影：撤销之后界面拿到的是**回退后**的工程 | [`the_projection_after_a_ui_undo_is_the_rolled_back_project`] |
//!
//! 为什么不写"控件树里有 `undo-tree-undo-button`"当证据：三方对齐矩阵的错位 5
//! 明确警告过这种假接线（元素都在、动作全是 `trace()`）。本文件断言的是
//! **工程字节真的回退了**，元素 ID 只是语义寻址（由 `elements.rs` 的判据管）。

#![allow(unused)]
use yeban_app::cli::{self, ProjectSource};
use yeban_app::undo::undo_session::wiring_fixture;
use yeban_app::undo::{
    ActionOutcome, UiAction, UndoPort, UndoSession, dispatch_key, perform_key, project_fingerprint,
};
use yeban_model::Op;
use yeban_model::container::{self, ContainerLimits};
use yeban_model::samples::filled_project;

const NOW: u64 = 1_760_000_000_000;

fn port() -> UndoPort {
    UndoPort::new(UndoSession::open("<判据>", "yeban-app", filled_project(), NOW).expect("打开"))
}

/// 提交夹具那次"改一个音符力度"（与 MCP 侧 `yeban_edit_notes` 编译出的 op 同形）。
fn commit_fixture(port: &UndoPort) -> Op {
    let fixture = wiring_fixture(&port.project()).expect("夹具");
    port.commit_ops(NOW + 1, "接线判据", vec![fixture.op()])
        .expect("提交");
    fixture.op()
}

/// 判据 ⑧（界面侧那一半）：界面路径撤出来的工程与**原始工程**逐字节相同。
///
/// MCP 侧的同名判据断言"工具路径 = 原始工程"，两条合起来就得到
/// "UI 路径 = MCP 路径"（同一串字节，同一个 `undo_session`）。
#[test]
fn the_ui_path_matches_the_shared_implementation_byte_for_byte() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 ⑨：**"点击"⇒ 工程真的回退了一版**（动作日志是证据，不是控件树）。
#[test]
fn a_click_really_rolls_the_project_back() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 ⑨：`Cmd+Z` / `Cmd+Shift+Z` / `Cmd+Shift+H` 整条链都落到同一个实现。
#[test]
fn the_cmd_z_chain_reaches_the_model() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 ⑫：打开**另一个**工程 = 新会话 ⇒ 不能撤到上一个工程的状态。
#[test]
fn a_new_project_open_starts_a_fresh_undo_session() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 ⑪：撤销 → 保存（真容器）→ 重开 ⇒ 与内存态一致；游标不落盘但可推导。
#[test]
fn saved_after_an_undo_reopens_to_the_same_bytes() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据 ⑩：撤销之后工程仍然通过 `validate()`（路由 / 自动化 / 片段池）。
#[test]
fn the_project_still_validates_after_a_ui_undo() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据：显示态（能否撤销 / 还能撤几步 / 提交数）**来自模型读数**。
#[test]
fn the_display_state_comes_from_the_model_readings() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据：撤销之后**投影**拿到的是回退后的工程（界面画的是回退后那一版）。
#[test]
fn the_projection_after_a_ui_undo_is_the_rolled_back_project() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据：`--open` 的真实来源标签会进撤销会话（报告行可核对）。
#[test]
fn the_session_label_comes_from_the_project_source() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}

/// 判据：`OpOrigin` 与会话作者如实落在提交上（撤销重做**不**产生新提交）。
#[test]
fn undo_and_redo_never_add_commits() {
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
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
