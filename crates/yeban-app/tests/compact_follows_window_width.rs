//! ⭐ 任务单 `#191`：**`compact` 断点跟着真实窗口宽走**。
//!
//! `#184` / `#189` 都逐字登记过同一个缺口：`host.rs:8451` 的
//! `ui.set_compact(scene.compact())` 读的是 `DemoScene::viewport_width`（恒 `1920`,
//! `scene.rs:188`），而 `DemoScene::compact()` 只在装配时算一次 ⇒ 用户把窗口拉窄，
//! 左栏**不会**自动折叠。
//!
//! 本判据只走**生产构造入口** [`host::build_main_window`]，不碰 `set_compact`：
//!
//! | 判据 | 读数 |
//! | :--- | :--- |
//! | ① 三档断点值 | `1920 ⇒ false`、`1440 ⇒ true`、`1024 ⇒ true`（`[UI-GRID-002]` 的断点是 `1920`, `scene.rs:50`） |
//! | ② 左栏宽随之变 | `240 → 36 → 36 → 240`（`Tokens.left-rail-width` / `left-rail-collapsed-width`） |
//! | ③ 节点在场性随之变 | 只在展开态出现的 `sidebar-search-field`：`true → false → false → true` |
//! | ④ 真窗口 resize | 同一实例 `1920 → 1440 → 1024 → 1920`，每次都是 `Tier1Window::resize`（= 平台适配器的 `WindowEvent::Resized`） |
//!
//! ## 它证什么 / 不证什么
//!
//! - 证：`MainWindow.compact` 是 `width < 1920px` 的绑定，**窗口尺寸真的变了**它就会翻；
//!   两次翻向（宽→窄、窄→宽）都测，⛔ 不是一次性的初值巧合。
//! - 证：折叠不只是 `compact` 这个布尔变了 —— 左栏几何与展开态节点**同时**跟着变。
//! - ⛔ 不证：真 winit 窗口被人类用手拖动。本机没有可交互的桌面会话，判据是
//!   **无头 Tier-1** 平台；`resize` 走的是 `MinimalSoftwareWindow::set_size` ⇒
//!   `WindowEvent::Resized`，与 OS 侧 resize 在 Slint 的 `WindowAdapter` 上是**同一条**
//!   事件。人类验收仍是"拉窗口看左栏是否折叠"（见交付报告 §10）。
//! - ⛔ 不证：1920 基准帧（见 `#191` 的 1920 帧哈希对照，不在本文件里）。

use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 生产窗口的唯一构造入口（`run_gui` 用的同一条）。
fn build(size: Size) -> LivePort<MainWindow> {
    let view = ViewState::demo();
    let scene = DemoScene::from_view(&view);
    LivePort::new(size, Permission::ReadOnly, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口")
}

fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

fn sidebar_width(tree: &ControlTree) -> u32 {
    tree.find_by_id("sidebar")
        .expect("`sidebar` 必须在场")
        .bounds
        .expect("可见控件必须有几何")
        .width
}

fn search_field_in_tree(tree: &ControlTree) -> bool {
    tree.contains("sidebar-search-field")
}

fn right_rail_count(tree: &ControlTree) -> usize {
    tree.ids().filter(|id| *id == "ai-rail").count()
}

/// 一次 resize ＋ 一次 `Resized` 事件后的**完整读数**。
///
/// 先 `capture()` 强制 Slint 跑一遍布局（与生产的事件循环同一条路径），然后重抓树；
/// 这样几何断言的读数不是"上一次布局"的陈旧值。
fn observe(port: &LivePort<MainWindow>, label: &str) -> (bool, u32, bool, usize) {
    port.window().capture().expect("resize 之后必须能重绘一帧");
    let tree = runtime_tree(port.ui());
    let compact = port.ui().get_compact();
    let sidebar = sidebar_width(&tree);
    let search = search_field_in_tree(&tree);
    let rails = right_rail_count(&tree);
    let window = port.window().size();
    report_line(&format!(
        "[#191 {label}] 窗口 {}×{} ⇒ compact={compact}；sidebar 宽={sidebar}；\
         `sidebar-search-field` 在场={search}；`ai-rail`={rails}",
        window.width, window.height
    ));
    (compact, sidebar, search, rails)
}

// ===========================================================================
// 判据 ①–④：同一实例上真 resize 扫三档，再回到 1920
// ===========================================================================
#[test]
fn the_compact_breakpoint_follows_the_real_window_width() {
    std::thread::spawn(|| {
        let port = build(Size::new(1920, 1080));

        // 1920：`[UI-GRID-002]` 的全展开档 ⇒ `width < 1920px` 求值为 false。
        let (compact, sidebar, search, rails) = observe(&port, "1920");
        assert!(
            !compact,
            "[#191 1920] 1920 宽必须落在全展开档（compact == false）"
        );
        assert_eq!(sidebar, 240, "[#191 1920] 展开态左栏必须是 240px");
        assert!(
            search,
            "[#191 1920] `sidebar-search-field` 在展开态必须在场"
        );
        assert_eq!(rails, 1, "[#191 1920] 展开态必须恰有 1 个常驻 `ai-rail`");

        // 1440：1366–1919 折叠档 ⇒ true（`1440 < 1920`）。
        port.window()
            .resize(Size::new(1440, 900))
            .expect("resize 1440");
        let (compact, sidebar, search, rails) = observe(&port, "1440");
        assert!(
            compact,
            "[#191 1440] 1440 宽必须落在折叠档（compact == true）"
        );
        assert_eq!(sidebar, 36, "[#191 1440] 折叠态左栏必须是 36px 图标导轨");
        assert!(
            !search,
            "[#191 1440] 折叠态 `sidebar-search-field` 必须离树"
        );
        assert_eq!(
            rails, 0,
            "[#191 1440] 折叠态常驻右栏离树（抽屉未打开时为 0）"
        );

        // 1024：规范未定义更窄档，实现取保守解 ⇒ 仍折叠。
        port.window()
            .resize(Size::new(1024, 800))
            .expect("resize 1024");
        let (compact, sidebar, search, rails) = observe(&port, "1024");
        assert!(
            compact,
            "[#191 1024] 1024 宽必须落在折叠档（compact == true）"
        );
        assert_eq!(sidebar, 36, "[#191 1024] 折叠态左栏必须是 36px");
        assert!(
            !search,
            "[#191 1024] 折叠态 `sidebar-search-field` 必须离树"
        );
        assert_eq!(rails, 0, "[#191 1024] 折叠态常驻右栏离树");

        // 再拉回 1920：⛔ 不是"只能折叠一次" —— 绑定必须能翻回来。
        port.window()
            .resize(Size::new(1920, 1080))
            .expect("resize 回 1920");
        let (compact, sidebar, search, rails) = observe(&port, "1920-again");
        assert!(!compact, "[#191 1920-again] 拉回 1920 必须回到展开档");
        assert_eq!(sidebar, 240, "[#191 1920-again] 左栏必须回到 240px");
        assert!(search, "[#191 1920-again] `sidebar-search-field` 必须回树");
        assert_eq!(rails, 1, "[#191 1920-again] 常驻 `ai-rail` 必须回树");
    })
    .join()
    .expect("判据线程");
}

// ===========================================================================
// 判据 ⑤：两个不同尺寸的窗口**出生**时就不同（⛔ 不靠"先建 1920 再 resize"）
// ===========================================================================
#[test]
fn windows_born_at_1024_and_1920_start_in_their_own_breakpoint() {
    let narrow = std::thread::spawn(|| {
        let port = build(Size::new(1024, 800));
        let (compact, sidebar, search, rails) = observe(&port, "born-1024");
        assert!(compact, "[#191 born-1024] 1024 窗口出生时必须 compact");
        assert_eq!(sidebar, 36, "[#191 born-1024] 左栏必须是 36px");
        assert!(!search, "[#191 born-1024] 展开态节点必须不在树里");
        assert_eq!(rails, 0, "[#191 born-1024] 常驻右栏必须不在树里");
    });
    let wide = std::thread::spawn(|| {
        let port = build(Size::new(1920, 1080));
        let (compact, sidebar, search, rails) = observe(&port, "born-1920");
        assert!(!compact, "[#191 born-1920] 1920 窗口出生时必须非 compact");
        assert_eq!(sidebar, 240, "[#191 born-1920] 左栏必须是 240px");
        assert!(search, "[#191 born-1920] 展开态节点必须在场");
        assert_eq!(rails, 1, "[#191 born-1920] 常驻右栏必须在场");
    });
    narrow.join().expect("1024 判据线程");
    wide.join().expect("1920 判据线程");
}
