//! **左侧资源栏条目选中**（`sidebar-item-0..7`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 资源栏的八个条目**不再**是"第 0 条永远高亮、按哪个都没反应"的静态列表：
//! 点其中一条，它把条目号交给宿主，宿主钳位后写 `MainWindow.sidebar-item`；
//! 选中态（背景、文字色、`accessible-item-selected`）三处读的都是这一个属性。
//! 纯视图态 —— 不改工程、不进撤销栈（`MODEL-ISO-001` 的挥发性层）。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（可寻址） | 八个条目都在运行时控件树里、角色 = `ListItem`、名称 = 条目名（逐字）；装配态 `sidebar-item == 0` |
//! | ②（真点击） | 注入一次真实指针点击到第 2 条 ⇒ 属性 `0 → 1`，八条几何全不动（视图态不重排布局） |
//! | ③（可见态真的换了） | 点击前后抓帧：第 1、2 条的像素**同时**变、第 3 条的像素**不变**；再点回第 1 条 ⇒ 逐位还原 |
//! | ④（越界钳位） | `invoke_select_sidebar_item(99)` ⇒ 7；`(-5)` ⇒ 0 —— 界面侧拿不到越界值 |
//! | ⑤（文本层接线） | `sidebar.slint` 的 `clicked` 与 `accessible-action-default` 同一落点；`app.slint` 原样转发；宿主是唯一钳位 / 写属性处 |
//!
//! ## 名称口径（⚠ 与分类判据不同的一处）
//!
//! 条目的**活标签**是资源名（`root.items[item_index]`，如 `"Studio Grand 三角钢琴"`），
//! 而 `elements.rs` 注册表里给 `sidebar-item-{i}` 登记的是合成标签 `"资源条目 {i}"`
//! （`test_port_adapter.rs` 的无障碍名判据在其文档里点名了这处差异：注册表的合成标签
//! 会**掩盖**活标签为空串的缺陷）。因此本判据只与注册表对账**角色**，名称则与
//! `sidebar.slint` 的 `items` 数组逐字对账。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/sidebar_category.rs` / `tests/console_maximize.rs` 同款）。

use yeban_app::bridge::ViewState;
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ `compact` 断点之外，资源栏是展开态）。
const WINDOW: Size = Size::new(1440, 900);

/// 八个条目的活标签（`ui/sidebar.slint` 的 `items` 数组逐字）。
const ITEM_LABELS: [&str; 8] = [
    "Studio Grand 三角钢琴",
    "Analog Brass 合成铜管",
    "Night Pad 夜色铺底",
    "Sub Bass 低频",
    "Drum Kit 909 鼓组",
    "String Ensemble 弦乐",
    "Tape Choir 磁带人声",
    "SFZ · 323 款采样预置",
];

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

/// 抓一帧，取某个条目内部一点的 RGB。
///
/// 采样点取「左内边距 6px、垂直居中」：避开圆角（`radius-sm`）与从 12px 开始的文字。
fn swatch_of(port: &LivePort<yeban_app::ui::MainWindow>, bounds: Rect) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    let x = u32::try_from(bounds.x + 6).expect("条目在窗口内");
    let y = bounds.y + i32::try_from(bounds.height / 2).expect("高度是正数");
    let y = u32::try_from(y).expect("条目在窗口内");
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
fn clicking_a_sidebar_item_moves_the_selection_and_the_highlight() {
    let (mut port, view) = live_window();

    // ---- ① 可寻址：八个条目在场，角色与注册表一致，名称 = 条目名 ----
    let registry = ElementRegistry::from_view(&view);
    let mut bounds = [Rect::new(0, 0, 0, 0); 8];
    for (index, expected_label) in ITEM_LABELS.iter().enumerate() {
        let id = format!("sidebar-item-{index}");
        let tree = port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在运行时控件树里"));
        assert_eq!(
            node.role.as_str(),
            ElementKind::ListItem.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        assert_eq!(
            node.label.as_str(),
            *expected_label,
            "屏读器读到的是条目**名**（`items` 数组逐字），⛔ 不是序号"
        );
        let meta = registry
            .get(&id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, ElementKind::ListItem);
        bounds[index] = bounds_of(&mut port, &id);
        assert!(
            bounds[index].width > 0 && bounds[index].height > 0,
            "`{id}` 必须有可见几何: {:?}",
            bounds[index]
        );
    }
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "装配态必须是第一条（默认帧与既有基准一位不变）"
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

    // ---- ② 真实点击第 2 条：属性翻过去，几何不动 ----
    click(&mut port, "sidebar-item-1", bounds[1]);
    assert_eq!(
        port.ui().get_sidebar_item(),
        1,
        "点击第 2 条之后 `sidebar-item` 必须是 1"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    for (index, expected) in bounds.iter().enumerate() {
        let id = format!("sidebar-item-{index}");
        assert_eq!(
            tree.find_by_id(&id).and_then(|node| node.bounds),
            Some(*expected),
            "`{id}` 的几何不该被视图态改动（条目选中不重排布局）"
        );
    }

    // ---- ③ 可见态真的换了：第 1、2 条同时变，第 3 条不变 ----
    let after = [
        swatch_of(&port, bounds[0]),
        swatch_of(&port, bounds[1]),
        swatch_of(&port, bounds[2]),
    ];
    assert_ne!(
        after[0], before[0],
        "第 1 条必须失去选中态（背景换回未选中色）: {before:?} → {after:?}"
    );
    assert_ne!(
        after[1], before[1],
        "第 2 条必须获得选中态（背景换成选中色）: {before:?} → {after:?}"
    );
    assert_eq!(
        after[2], before[2],
        "第 3 条与这次点击无关，像素必须一位不变: {before:?} → {after:?}"
    );
    assert_eq!(
        (after[0], after[1]),
        (before[1], before[0]),
        "选中色必须**搬家**（0 拿到 1 原来的颜色、1 拿到 0 原来的颜色），而不是第三种颜色"
    );

    // ---- 再点回第 1 条：属性与像素都逐位还原 ----
    click(&mut port, "sidebar-item-0", bounds[0]);
    assert_eq!(port.ui().get_sidebar_item(), 0);
    let restored = [
        swatch_of(&port, bounds[0]),
        swatch_of(&port, bounds[1]),
        swatch_of(&port, bounds[2]),
    ];
    assert_eq!(
        restored, before,
        "点回第 1 条之后三个条目的像素必须逐位回到装配态"
    );

    // ---- ④ 越界钳位：界面侧拿不到越界值 ----
    port.ui().invoke_select_sidebar_item(99);
    assert_eq!(
        port.ui().get_sidebar_item(),
        7,
        "越界值 99 必须被宿主钳到最后一条"
    );
    port.ui().invoke_select_sidebar_item(-5);
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "越界值 -5 必须被宿主钳到第一条"
    );

    // ---- ⑤ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let sidebar =
        std::fs::read_to_string(manifest_dir.join("ui/sidebar.slint")).expect("读 sidebar.slint");
    assert!(
        sidebar.contains("accessible-action-default => { root.select-item(item_index); }"),
        "条目必须有无障碍动作"
    );
    assert!(
        sidebar.contains("clicked => { root.select-item(item_index); }"),
        "条目必须有点击源，且落点与无障碍动作逐字相同"
    );
    assert!(
        sidebar.contains("in property <int> active-item: 0;")
            && sidebar.contains("accessible-item-selected: item_index == root.active-item;")
            && sidebar.contains("background: item_index == root.active-item ?"),
        "选中态必须唯一读 `active-item`"
    );
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    assert!(
        app.contains("callback select-sidebar-item(int);")
            && app.contains("active-item: root.sidebar-item;")
            && app.contains("root.select-sidebar-item(item);"),
        "主窗口必须把条目点击原样转给宿主，并把属性下发给资源栏"
    );
    let host_src = std::fs::read_to_string(manifest_dir.join("src/host.rs")).expect("读 host.rs");
    assert!(
        host_src.contains("ui.on_select_sidebar_item(")
            && host_src.contains("item.clamp(0, SIDEBAR_ITEM_COUNT - 1)"),
        "宿主必须是唯一的钳位 / 写属性处"
    );

    report_line(&format!(
        "[sidebar-item] 第 1 条的选中背景 {:?} / 未选中背景 {:?}；\
         点击第 2 条后属性 0 → {}；越界 99 → 7、-5 → 0",
        before[0], before[1], 1
    ));
}
