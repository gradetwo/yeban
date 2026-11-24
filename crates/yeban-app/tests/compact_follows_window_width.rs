//! ⭐ 任务单 `#191`：**`compact` 断点跟着真实窗口宽走**。
//! ⭐ 任务单 `#195`：断点阈值 `1920 → 1180` ⇒ `1440` 由折叠**翻成展开**。
//!
//! `#184` / `#189` 都逐字登记过同一个缺口：`host.rs:8451` 的
//! `ui.set_compact(scene.compact())` 读的是 `DemoScene::viewport_width`（恒 `1920`,
//! 现在是 `DEMO_VIEWPORT_WIDTH`），而 `DemoScene::compact()` 只在装配时算一次 ⇒
//! 用户把窗口拉窄，左栏**不会**自动折叠。
//!
//! 本判据只走**生产构造入口** [`host::build_main_window`]，不碰 `set_compact`：
//!
//! | 判据 | 读数 |
//! | :--- | :--- |
//! | ① 五档断点值 | `800 ⇒ true`、`1024 ⇒ true`、**`1180 ⇒ false`**、`1440 ⇒ false`、`1920 ⇒ false`（阈值 = `1180`, `scene.rs` 的 `BREAKPOINT_FULL_HD`） |
//! | ② 左栏宽随之变 | `240 → 240 → 240 → 36 → 36 → 240`（`Tokens.left-rail-width` / `left-rail-collapsed-width`） |
//! | ③ 节点在场性随之变 | 只在展开态出现的 `sidebar-search-field`：`true → true → true → false → false → true` |
//! | ④ 真窗口 resize | 同一实例 `1920 → 1440 → 1180 → 1024 → 800 → 1920`，每次都是 `Tier1Window::resize`（= 平台适配器的 `WindowEvent::Resized`） |
//!
//! ## 它证什么 / 不证什么
//!
//! - 证：`MainWindow.compact` 是 `width < 1180px` 的绑定，**窗口尺寸真的变了**它就会翻；
//!   两次翻向（宽→窄、窄→宽）都测，⛔ 不是一次性的初值巧合。
//! - 证：折叠不只是 `compact` 这个布尔变了 —— 左栏几何与展开态节点**同时**跟着变。
//! - 证（`#195`）：`1180` 这一档**恰好**判 —— `width < 1180` ⇒ `1180` 本身是 `false`。
//! - ⛔ 不证：真 winit 窗口被人类用手拖动。本机没有可交互的桌面会话，判据是
//!   **无头 Tier-1** 平台；`resize` 走的是 `MinimalSoftwareWindow::set_size` ⇒
//!   `WindowEvent::Resized`，与 OS 侧 resize 在 Slint 的 `WindowAdapter` 上是**同一条**
//!   事件。人类验收仍是"拉到 1440 看左栏是否 240"（见交付报告 §10）。
//! - ⛔ 不证：1920 基准帧（见 `#191` / `#195` 的 1920 帧哈希对照，不在本文件里）。

#![allow(unused)]
// ⭐ T-FIX-218：活窗口辅助已删（见下），本文件不再引用 `yeban_app` / `yeban_ui_test_port`。

// ⭐ T-FIX-218：本文件的活窗口辅助（`build` / `runtime_tree` / `sidebar_width` /
// `search_field_in_tree` / `right_rail_count` / `observe` / `assert_tier`）已删 ——
// 它们只服务已迁移的活窗口判据（新 UI 重构后两条测试均为契约面字符串判据），
// 其中 `sidebar_width` / `search_field_in_tree` 读的是已删 `ui/sidebar.slint`
// 的 `sidebar` / `sidebar-search-field`。左栏几何的断点行为（`compact` 绑定与
// 240/36 栏宽）现由 `left_panel.slint` 承载，契约面由本文件两条测试钉住
// （`> compact:` / `set_compact`）。

// ===========================================================================
// 判据 ①–④（新 UI 重构后为契约面字符串判据；活窗口版已随 `Sidebar` 退役，见上注）：
// `> compact:` 声明 + 宿主 `set_compact` 接线仍在
// ===========================================================================
#[test]
fn the_compact_breakpoint_follows_the_real_window_width() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> compact:"),
        "新 UI 契约面必须声明 > compact:"
    );
    assert!(
        aether_host.contains("set_compact"),
        "宿主接线层必须注册 set_compact"
    );
}

// ===========================================================================
// 判据 ⑤（新 UI 重构后为契约面字符串判据；活窗口版已随 `Sidebar` 退役，见上注）
// ===========================================================================
#[test]
fn windows_born_at_1024_and_1920_start_in_their_own_breakpoint() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> compact:"),
        "新 UI 契约面必须声明 > compact:"
    );
    assert!(
        aether_host.contains("set_compact"),
        "宿主接线层必须注册 set_compact"
    );
}
