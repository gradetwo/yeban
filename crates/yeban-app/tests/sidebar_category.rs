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

use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ `compact` 断点之外，资源栏是展开态）。
const WINDOW: Size = Size::new(1440, 900);

/// 三个分类的语义名（与 `elements.rs` 注册表逐字一致）。
const CATEGORY_LABELS: [&str; 3] = ["资源分类 乐器", "资源分类 采样", "资源分类 预置"];

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` 与无头装配调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    // ⭐ 分类 / 条目点击现在住在 `host::wire_sidebar_contents`（换分类 ⇒ 条目数组跟着换，
    // 需要一份投影）。判据接的是**同一个**函数，⛔ 不是另一条捷径。
    host::wire_sidebar_contents(
        port.ui(),
        std::rc::Rc::new(std::cell::RefCell::new(view.clone())),
    );
    (port, view)
}

fn bounds_of(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str) -> Rect {
    port.refresh_tree(None)
        .expect("必须能重抓控件树")
        .find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

/// 抓一帧，取某个标签内部一点的 RGB。
///
/// 采样点取「左内边距 6px、垂直居中」：避开圆角（`radius-sm`）与居中文字。
fn swatch_of(port: &LivePort<yeban_app::ui::MainWindow>, bounds: Rect) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    let x = u32::try_from(bounds.x + 6).expect("分类标签在窗口内");
    let y = bounds.y + i32::try_from(bounds.height / 2).expect("高度是正数");
    let y = u32::try_from(y).expect("分类标签在窗口内");
    image
        .pixel(x, y)
        .unwrap_or_else(|| panic!("采样点 ({x}, {y}) 必须在 {WINDOW:?} 帧内"))
}

fn click(port: &mut LivePort<yeban_app::ui::MainWindow>, id: &str, bounds: Rect) {
    let x = f64::from(bounds.width) / 2.0;
    let y = f64::from(bounds.height) / 2.0;
    port.dispatch_pointer_down(id, x, y, PointerButton::Left)
        .expect("指针按下必须落到活窗口");
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
}

#[test]
fn clicking_a_sidebar_category_switches_the_view_state_and_the_highlight() {
    let (mut port, view) = live_window();

    // ---- ① 可寻址：三个标签在场，角色 / 名称与注册表一致 ----
    let registry = ElementRegistry::from_view(&view);
    let mut bounds = [Rect::new(0, 0, 0, 0); 3];
    for (index, expected_label) in CATEGORY_LABELS.iter().enumerate() {
        let id = format!("sidebar-category-{index}-button");
        let tree = port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在运行时控件树里"));
        assert_eq!(
            node.role.as_str(),
            ElementKind::Tab.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        assert_eq!(
            node.label.as_str(),
            *expected_label,
            "屏读器读到的是分类**名**（⛔ 不是序号），且与注册表逐字一致"
        );
        let meta = registry
            .get(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, ElementKind::Tab);
        assert_eq!(meta.label, *expected_label);
        bounds[index] = bounds_of(&mut port, &id);
        assert!(
            bounds[index].width > 0 && bounds[index].height > 0,
            "`{id}` 必须有可见几何: {:?}",
            bounds[index]
        );
    }
    assert_eq!(
        port.ui().get_sidebar_category(),
        0,
        "装配态必须是第一个分类（默认帧与既有基准一位不变）"
    );

    let before = [
        swatch_of(&port, bounds[0]),
        swatch_of(&port, bounds[1]),
        swatch_of(&port, bounds[2]),
    ];
    assert_ne!(
        before[0], before[1],
        "前提：选中色与未选中色必须不同（否则下面的像素断言证明不了任何东西）"
    );

    // ---- ② 真实点击「采样」：属性翻过去，几何不动 ----
    click(&mut port, "sidebar-category-1-button", bounds[1]);
    assert_eq!(
        port.ui().get_sidebar_category(),
        1,
        "点击「采样」之后 `sidebar-category` 必须是 1"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    for (index, id) in [
        "sidebar-category-0-button",
        "sidebar-category-1-button",
        "sidebar-category-2-button",
    ]
    .iter()
    .enumerate()
    {
        assert_eq!(
            tree.find_by_id(id).and_then(|node| node.bounds),
            Some(bounds[index]),
            "`{id}` 的几何不该被视图态改动（分类切换不重排布局）"
        );
    }

    // ---- ③ 可见态真的换了：0 与 1 同时变，2 不变 ----
    let after = [
        swatch_of(&port, bounds[0]),
        swatch_of(&port, bounds[1]),
        swatch_of(&port, bounds[2]),
    ];
    assert_ne!(
        after[0], before[0],
        "「乐器」必须失去选中态（背景换回未选中色）: {before:?} → {after:?}"
    );
    assert_ne!(
        after[1], before[1],
        "「采样」必须获得选中态（背景换成选中色）: {before:?} → {after:?}"
    );
    assert_eq!(
        after[2], before[2],
        "「预置」与这次点击无关，像素必须一位不变: {before:?} → {after:?}"
    );
    assert_eq!(
        (after[0], after[1]),
        (before[1], before[0]),
        "选中色必须**搬家**（0 拿到 1 原来的颜色、1 拿到 0 原来的颜色），而不是第三种颜色"
    );

    // ---- 再点回「乐器」：属性与像素都逐位还原 ----
    click(&mut port, "sidebar-category-0-button", bounds[0]);
    assert_eq!(port.ui().get_sidebar_category(), 0);
    let restored = [
        swatch_of(&port, bounds[0]),
        swatch_of(&port, bounds[1]),
        swatch_of(&port, bounds[2]),
    ];
    assert_eq!(
        restored, before,
        "点回「乐器」之后三个分类的像素必须逐位回到装配态"
    );

    // ---- ④ 越界钳位：界面侧拿不到越界值 ----
    port.ui().invoke_select_sidebar_category(7);
    assert_eq!(
        port.ui().get_sidebar_category(),
        2,
        "越界值 7 必须被宿主钳到最后一个分类"
    );
    port.ui().invoke_select_sidebar_category(-3);
    assert_eq!(
        port.ui().get_sidebar_category(),
        0,
        "越界值 -3 必须被宿主钳到第一个分类"
    );

    // ---- ⑤ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sidebar =
        std::fs::read_to_string(manifest_dir.join("ui/sidebar.slint")).expect("读 sidebar.slint");
    assert!(
        sidebar.contains("accessible-action-default => { root.select-category(category_index); }"),
        "分类标签必须有无障碍动作"
    );
    assert!(
        sidebar.contains("clicked => { root.select-category(category_index); }"),
        "分类标签必须有点击源，且落点与无障碍动作逐字相同"
    );
    assert!(
        sidebar.contains("in property <int> active-category: 0;")
            && sidebar
                .contains("accessible-item-selected: category_index == root.active-category;")
            && sidebar.contains("background: category_index == root.active-category ?"),
        "选中态必须唯一读 `active-category`"
    );
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    assert!(
        app.contains("callback select-sidebar-category(int);")
            && app.contains("active-category: root.sidebar-category;")
            && app.contains("root.select-sidebar-category(category);"),
        "主窗口必须把分类点击原样转给宿主，并把属性下发给资源栏"
    );
    let host_src = std::fs::read_to_string(manifest_dir.join("src/host.rs")).expect("读 host.rs");
    assert!(
        host_src.contains("ui.on_select_sidebar_category(")
            && host_src.contains("category.clamp(0, SIDEBAR_CATEGORY_COUNT - 1)"),
        "宿主必须是唯一的钳位 / 写属性处"
    );

    report_line(&format!(
        "[sidebar-category] 分类 0..=2 的选中背景 {:?} / 未选中背景 {:?}；\
         点击「采样」后属性 0 → {}；越界 7 → 2、-3 → 0",
        before[0], before[1], 1
    ));
}
