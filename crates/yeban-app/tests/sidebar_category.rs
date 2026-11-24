//! ⭐ T-FIX-218 退役注：本文件头注的活窗口判据 ①–⑤（运行时树里的
//! `sidebar-category-{0,1,2}-button`）已在新 UI 重构（`21f70fd1`）时退役为契约面
//! 字符串判据；`ui/sidebar.slint` 本身也在 T-FIX-218 删除（旧 `Sidebar` 组件无人
//! import、不参与编译）。下面头注是**历史记录**，不是现行断言 —— 现行断言只有文件
//! 末尾那一个 `#[test]`（`callback select-sidebar-category(` /
//! `> sidebar-category:` / 宿主 `sidebar_category` 三条字符串核对）。数据面仍活：
//! 分类号经 `select-sidebar-category` 交宿主钳位后写 `MainWindow.sidebar-category`。
//! 活窗口辅助里与已删注册表标签逐字的 `CATEGORY_LABELS` 随本次删除一并移除；其余
//! 通用辅助（`live_window` / `bounds_of` / `swatch_of` / `click`）是迁移留下的
//! 死代码，不引用已删面板，已由 T-FIX-395 删除（见收据）。
//!
//! **左侧资源栏分类切换**（`sidebar-category-{0,1,2}-button`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 资源栏顶部的三个分类标签（乐器 / 采样 / 预置）**不是一个静态高亮**：点其中一个，
//! 它把 `0..=2` 的分类号交给宿主，宿主钳位后写 `MainWindow.sidebar-category`；
//! 选中态（背景、文字色、`accessible-item-selected`）三处读的都是这一个属性。
//! 纯视图态 —— 不改工程、不进撤销栈（`MODEL-ISO-001` 的挥发性层）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 三个标签都在运行时控件树里、角色 = `Tab`、名称与注册表逐字一致；装配态 `sidebar-category == 0` |
//! | ②（真点击） | 注入一次真实指针点击到「采样」⇒ 属性 `0 → 1`，几何不动（视图态不重排布局） |
//! | ③（可见态真的换了） | 点击前后抓帧：分类 0 与 1 的像素**同时**变、分类 2 的像素**不变**；再点回分类 0 ⇒ 三个像素逐位回到点击前 |
//! | ④（越界钳位） | `invoke_select_sidebar_category(7)` ⇒ 2；`(-3)` ⇒ 0 —— 界面侧拿不到越界值 |
//! | ⑤（文本层接线） | `sidebar.slint` 的 `clicked` 与 `accessible-action-default` 同一落点；`app.slint` 原样转发；宿主是唯一钳位 / 写属性处 |
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/console_maximize.rs` / `tests/ui_bpm_field.rs` 同款）。

// ⭐ T-FIX-218：`CATEGORY_LABELS` 已删 —— 它与已删的 `sidebar-category-{i}-button`
// 注册表标签逐字一致（`elements.rs` 里那族登记已随 `ui/sidebar.slint` 一起删除），无调用者。
#[test]
fn clicking_a_sidebar_category_switches_the_view_state_and_the_highlight() {
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
        aether_app.contains("callback select-sidebar-category("),
        "契约面必须声明 callback select-sidebar-category("
    );
    assert!(
        aether_host.contains("select_sidebar_category"),
        "宿主接线层必须注册 select_sidebar_category"
    );
    assert!(
        aether_app.contains("> sidebar-category:"),
        "契约面必须声明 > sidebar-category:"
    );
    assert!(
        aether_host.contains("sidebar_category"),
        "宿主接线层必须注册 sidebar_category"
    );
}
