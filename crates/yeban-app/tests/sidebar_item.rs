//! **左侧资源栏条目 = 投影，选中态 = 视图态** —— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 资源栏的条目**不再**是 `.slint` 里内联的 8 个演示名（含 `"SFZ · 323 款采样预置"`
//! 这种仓里不存在的数字）。它由宿主按**当前分类**注入，取数口是
//! [`bridge::sidebar_category_content`]（乐器 = 采样库清单，采样 = 工程素材池，
//! 预置 = 空 + 如实理由）。于是三件事必须同时成立：
//!
//! 1. 画面的条目**条数 / 顺序 / 名称**逐字等于那个投影 —— 库里有 3 份就是 3 条，
//!    ⛔ 不是"永远画 8 条"；
//! 2. 点其中一条，它把条目号交给宿主，宿主**按实际条数**钳位后写
//!    `MainWindow.sidebar-item`；选中态（背景、文字色、`accessible-item-selected`）
//!    三处读的都是这一个属性（纯视图态，`MODEL-ISO-001` 的挥发性层：不改工程、
//!    不进撤销栈）；
//! 3. 换分类 ⇒ 宿主在**同一次**里重注入数组并把选中态落回合法范围；投影为空时
//!    画的是**一句理由**（`sidebar-empty`），⛔ 不是 8 条幽灵、⛔ 不是空白。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（条数与名称同源） | 假库根里装 3 份乐器 ⇒ 树里**恰好** 3 条 `sidebar-item-{0,1,2}`，标签 = 清单 `name` 逐字、顺序 = 清单顺序；第 4 条与 `sidebar-empty` **不在场**；注册表与画面同源同标签 |
//! | ②（真点击） | 注入一次真实指针点击到第 2 条 ⇒ 属性 `0 → 1`，三条几何全不动；第 1、2 条像素**同时**变、第 3 条**不变**；点回第 1 条 ⇒ 逐位还原 |
//! | ③（钳位按实际条数） | `invoke_select_sidebar_item(99)` ⇒ **2**（= `len-1`，⛔ 不是常量 7）；`(-5)` ⇒ 0 |
//! | ④（换分类 = 换取数口） | 切到分类 1（采样）⇒ 条数与标签变成 `view.asset_pool_names()`，`sidebar-empty` 不在场 |
//! | ⑤（空态诚实） | 切到分类 2（预置，模型无载体 ⇒ 恒空）⇒ 条目族**全部离场**、`sidebar-empty` 在场且标签逐字 = [`bridge::SIDEBAR_EMPTY_PRESETS_REASON`]、`sidebar-item` 落回 0 |
//! | ⑥（文本层接线） | `sidebar.slint` 的 `clicked` 与 `accessible-action-default` 同一落点；条目数组**没有**内联默认值；`app.slint` 转发数组与文案；宿主是唯一钳位 / 写属性处，且钳位表达式读**投影条数**（⛔ 不是 `SIDEBAR_ITEM_COUNT` 那类常量） |
//!
//! ## 为什么名称口径与旧版不同
//!
//! 旧版在这里把 8 个演示名当"活标签"手抄了一遍，而 `elements.rs` 给 `sidebar-item-{i}`
//! 登记的是合成标签 `"资源条目 {i}"` —— 两份清单各写一份，谁也不对账。现在注册表与
//! 画面**同源**（同一个 `sidebar_category_content`），所以本判据同时钉两侧：名称逐字
//! 相等，而且**都是投影给的**。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒
//! 本文件只有一个 `#[test]`（与 `tests/sidebar_category.rs` / `tests/console_maximize.rs` 同款）。
//! [`SAMPLE_LIBRARY_ENV`] 同样**每进程一次**：写发生在 `OnceLock` 初始化里，任何库根
//! 读取都发生在本函数返回之后（Rust 2024 `set_var` 的安全性前提，与
//! `tests/track_instrument_switch.rs` 同一条纪律）。

use std::path::PathBuf;
use std::sync::OnceLock;

use yeban_app::bridge::{self, ViewState};
use yeban_app::elements::{ElementKind, ElementRegistry};
use yeban_app::host;
use yeban_app::samples::{MANIFEST_RELATIVE, SAMPLE_LIBRARY_ENV};
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款（1440 宽 ⇒ `compact` 断点之外，资源栏是展开态）。
const WINDOW: Size = Size::new(1440, 900);

/// 夹具乐器：`assets/samples/manifest.json` 里的三个 `id`（逐字）。
///
/// ⚠ 只造 **SFZ 文本**、不造样本字节 —— 分类 0 的取数口只回答"库里有这份 SFZ"
/// （`scan_library_instruments` ⛔ 不解码样本，那是 `#210` 的裁决）。
const FIXTURE_IDS: [&str; 3] = ["aliexpress-erhu", "body-percussion", "cithara-barbarica"];

/// 一份夹具乐器：`(清单 name, 仓内 SFZ 路径, 库内 SFZ 路径)` —— 后两条分别相对仓库根
/// 与相对库根（两者只差 `assets/samples/` 前缀，见 `#196` 的落盘规则）。
struct Fixture {
    name: String,
    repository: String,
    library: String,
}

/// 夹具按**清单顺序**返回；id 不在清单里 ⇒ 响亮（⛔ 不静默少造一份）。
fn fixtures() -> &'static Vec<Fixture> {
    static FIXTURES: OnceLock<Vec<Fixture>> = OnceLock::new();
    FIXTURES.get_or_init(|| {
        let manifest = yeban_app::sfz_load::repo_root().join(MANIFEST_RELATIVE);
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|error| panic!("读 {}: {error}", manifest.display()));
        let document: serde_json::Value = serde_json::from_str(&text).expect("清单必须是合法 JSON");
        let entries = document
            .get("instruments")
            .and_then(|value| value.as_array())
            .expect("清单必须有 instruments 数组");
        let mut picked: Vec<Fixture> = Vec::new();
        for entry in entries {
            let id = entry
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or("");
            if !FIXTURE_IDS.contains(&id) {
                continue;
            }
            let name = entry
                .get("name")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| panic!("夹具乐器 {id} 必须有 name"));
            let relative_root = entry
                .get("relative_root")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| panic!("夹具乐器 {id} 必须有 relative_root"));
            let sfz = entry
                .get("sfz")
                .and_then(|value| value.as_str())
                .unwrap_or_else(|| panic!("夹具乐器 {id} 必须有 sfz"));
            // 库内布局剥掉 `assets/samples/` 前缀（`resolve_sampler_path` 的 `#196`
            // 规则，与下载脚本 `--dest` 同一条）⇒ 夹具必须按同一条规则落盘。
            let in_library = relative_root
                .strip_prefix("assets/samples/")
                .unwrap_or(relative_root)
                .trim_end_matches('/');
            picked.push(Fixture {
                name: name.to_owned(),
                repository: format!("{relative_root}/{sfz}"),
                library: format!("{in_library}/{sfz}"),
            });
        }
        assert_eq!(
            picked.len(),
            FIXTURE_IDS.len(),
            "清单里必须恰好找得到这 3 个 id，实际 {} 份",
            picked.len()
        );
        picked
    })
}

/// 假库根（进程内建一次）：把 3 份 SFZ 文本按**库内布局**落盘，再把
/// [`SAMPLE_LIBRARY_ENV`] 指过去。
fn install_fake_library() {
    static LIBRARY: OnceLock<PathBuf> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let repo = yeban_app::sfz_load::repo_root();
        let dir =
            std::env::temp_dir().join(format!("yeban-sidebar-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建假库根");
        for fixture in fixtures() {
            let source = repo.join(&fixture.repository);
            let text = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("读仓库 SFZ {}: {error}", source.display()));
            let target = dir.join(&fixture.library);
            let parent = target.parent().expect("SFZ 有父目录");
            std::fs::create_dir_all(parent).expect("建库内乐器目录");
            std::fs::write(target, text).expect("写库内 SFZ");
        }
        // SAFETY: 唯一一次写，发生在 `OnceLock` 初始化里 ⇒ 任何库根读取都在其后。
        unsafe { std::env::set_var(SAMPLE_LIBRARY_ENV, &dir) };
        dir
    });
}

fn live_window() -> (LivePort<yeban_app::ui::MainWindow>, ViewState) {
    install_fake_library();
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` 与无头装配调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    // ⭐ 条目 / 分类的**取数**接线：`host::wire_sidebar_contents`（钳位读当次投影条数）。
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

/// 抓一帧，取某个条目内部一点的 RGB。
///
/// 采样点取「左内边距 3px、垂直居中」：条目文字从 `space-3`（6px）开始左对齐，
/// 垂直中心远离 `radius-sm`（2px）圆角 ⇒ 该点只可能是纯背景色。
/// ⚠ 采样点必须避开**文字**：字形覆盖率随平台系统字体而变（CI run `40969319851`
/// 实测：条目 0 的首字形在 `x+6` 处有抗锯齿像素，踩上文字就把字体栅格差异带进了
/// 「选中色搬家」等式）。
fn swatch_of(port: &LivePort<yeban_app::ui::MainWindow>, bounds: Rect) -> [u8; 3] {
    let image = port.window().capture().expect("Tier-1 截图");
    let x = u32::try_from(bounds.x + 3).expect("条目在窗口内");
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

/// 树里**在场**的条目（`(id, 无障碍名)`，按 id 下标升序）。
///
/// ⚠ 数的是**运行时树**，⛔ 不是注册表、⛔ 不是任何常量 —— 本判据要证的正是"画面
/// 跟着投影走"。下标必须连续（0..条数），否则说明有条目被跳过或残留。
fn present_items(port: &mut LivePort<yeban_app::ui::MainWindow>) -> Vec<(String, String)> {
    let tree = port.refresh_tree(None).expect("重抓");
    let mut found: Vec<(usize, String, String)> = Vec::new();
    for node in tree.iter() {
        let Some(rest) = node.id.strip_prefix("sidebar-item-") else {
            continue;
        };
        let Ok(index) = rest.parse::<usize>() else {
            continue;
        };
        found.push((index, node.id.clone(), node.label.clone()));
    }
    found.sort_by_key(|(index, _, _)| *index);
    found
        .into_iter()
        .enumerate()
        .map(|(position, (index, id, label))| {
            assert_eq!(
                position, index,
                "条目 id 必须是**连续**下标（0..条数）：第 {position} 位实际是 `{id}`"
            );
            (id, label)
        })
        .collect()
}

#[test]
fn sidebar_items_are_the_projection_and_clicking_one_moves_only_the_view_state() {
    let (mut port, view) = live_window();

    // ---- ① 条数与名称同源（分类 0 = 乐器 = 假库里的 3 份） ----
    let fixtures = fixtures();
    let content = bridge::sidebar_category_content(&view, 0);
    assert_eq!(
        content.len(),
        fixtures.len(),
        "取数口必须给出夹具的全部 3 份（⛔ 不是内联的 8 条）: {:?}",
        content.items
    );
    for (index, fixture) in fixtures.iter().enumerate() {
        assert_eq!(
            content.items[index], fixture.name,
            "第 {index} 条必须按**清单顺序**携带清单的 name"
        );
    }
    assert_eq!(
        content.empty_reason, "",
        "① 非空分类不许带空态文案（`.slint` 的 `visible` 与它是同一判据）"
    );
    let registry = ElementRegistry::from_view(&view);
    let present = present_items(&mut port);
    assert_eq!(
        present.len(),
        content.len(),
        "① 画面条数必须等于投影条数，实际 {present:?}"
    );
    let mut bounds: Vec<Rect> = Vec::new();
    for (index, (id, label)) in present.iter().enumerate() {
        let tree = port.refresh_tree(None).expect("重抓");
        let node = tree
            .find_by_id(id)
            .unwrap_or_else(|| panic!("`{id}` 必须在运行时控件树里"));
        assert_eq!(
            node.role.as_str(),
            ElementKind::ListItem.accessible_role(),
            "`{id}` 的角色必须与注册表一致"
        );
        assert_eq!(
            label, &content.items[index],
            "屏读器读到的是投影的**名**，⛔ 不是序号"
        );
        let meta = registry
            .get(id)
            .unwrap_or_else(|| panic!("`{id}` 必须在装配时的注册表里（[UI-TEST-001]）"));
        assert_eq!(meta.kind, ElementKind::ListItem);
        assert_eq!(
            meta.label, content.items[index],
            "注册表标签必须与画面**逐字**相同（同源，⛔ 不是合成标签 `资源条目 {{i}}`）"
        );
        bounds.push(bounds_of(&mut port, id));
        assert!(
            bounds[index].width > 0 && bounds[index].height > 0,
            "`{id}` 必须有可见几何: {:?}",
            bounds[index]
        );
    }
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("sidebar-empty").is_none(),
        "① 有条目时**不许**画空态"
    );
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "装配态必须是第一条（默认帧与既有基准一位不变）"
    );

    // ---- ② 真实点击第 2 条：属性翻过去，几何不动，选中色搬家 ----
    let before: Vec<[u8; 3]> = bounds
        .iter()
        .take(3)
        .map(|entry| swatch_of(&port, *entry))
        .collect();
    assert_ne!(
        before[0], before[1],
        "前提：选中色与未选中色必须不同（否则像素断言证明不了任何东西）"
    );

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
    let after: Vec<[u8; 3]> = bounds
        .iter()
        .take(3)
        .map(|entry| swatch_of(&port, *entry))
        .collect();
    assert_ne!(
        after[0], before[0],
        "第 1 条必须失去选中态: {before:?} → {after:?}"
    );
    assert_ne!(
        after[1], before[1],
        "第 2 条必须获得选中态: {before:?} → {after:?}"
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
    click(&mut port, "sidebar-item-0", bounds[0]);
    assert_eq!(port.ui().get_sidebar_item(), 0);
    let restored: Vec<[u8; 3]> = bounds
        .iter()
        .take(3)
        .map(|entry| swatch_of(&port, *entry))
        .collect();
    assert_eq!(
        restored, before,
        "点回第 1 条之后三个条目的像素必须逐位回到装配态"
    );

    // ---- ③ 钳位按**实际条数**（越界 99 ⇒ `len-1 = 2`，⛔ 不是常量 7） ----
    port.ui().invoke_select_sidebar_item(99);
    assert_eq!(
        port.ui().get_sidebar_item(),
        content.len() as i32 - 1,
        "③ 越上界必须钳到最后一条（分类 0 共 {} 条）",
        content.len()
    );
    port.ui().invoke_select_sidebar_item(-5);
    assert_eq!(port.ui().get_sidebar_item(), 0, "③ 越下界必须钳到第一条");

    // ---- ④ 换分类 = 换取数口（采样 = 工程素材池的投影） ----
    let assets = view.asset_pool_names();
    assert!(
        !assets.is_empty(),
        "④ 夹具工程的素材池必须非空，否则这条判据什么也没证"
    );
    port.ui().invoke_select_sidebar_category(1);
    assert_eq!(port.ui().get_sidebar_category(), 1, "④ 宿主必须落位分类 1");
    let present = present_items(&mut port);
    assert_eq!(
        present.len(),
        assets.len(),
        "④ 分类 1 的条数必须换成素材池条数（{assets:?}），实际 {present:?}"
    );
    for (index, (_, label)) in present.iter().enumerate() {
        assert_eq!(
            label, &assets[index],
            "④ 分类 1 的名称必须逐字来自 `asset_pool_names()`"
        );
    }
    assert!(
        port.ui().get_sidebar_item() < assets.len() as i32,
        "④ 换分类后选中态必须落回**新**条数的合法范围，实际 {}",
        port.ui().get_sidebar_item()
    );
    let tree = port.refresh_tree(None).expect("重抓");
    assert!(
        tree.find_by_id("sidebar-empty").is_none(),
        "④ 非空分类不许画空态"
    );

    // ---- ⑤ 空态诚实（分类 2 = 预置：模型无载体 ⇒ 恒空 + 一句理由） ----
    port.ui().invoke_select_sidebar_category(2);
    assert_eq!(port.ui().get_sidebar_category(), 2, "⑤ 宿主必须落位分类 2");
    let preset = bridge::sidebar_category_content(&view, 2);
    assert!(
        preset.is_empty(),
        "⑤ 预置分类必须为空（模型没有载体），实际 {:?}",
        preset.items
    );
    assert!(
        present_items(&mut port).is_empty(),
        "⑤ 换到空分类 ⇒ 条目族必须**全部**离场"
    );
    let tree = port.refresh_tree(None).expect("重抓");
    let empty = tree
        .find_by_id("sidebar-empty")
        .expect("⑤ 空分类必须画出理由节点（⛔ 不是空白）");
    assert_eq!(
        empty.label,
        bridge::SIDEBAR_EMPTY_PRESETS_REASON,
        "⑤ 理由必须逐字 = 登记的模型缺口文案"
    );
    assert_eq!(
        empty.role.as_str(),
        ElementKind::Text.accessible_role(),
        "⑤ 空态是只读文本，⛔ 不是可点条目"
    );
    assert_eq!(
        port.ui().get_sidebar_item(),
        0,
        "⑤ 空分类的选中态必须落在哨兵 0（宿主是唯一写者）"
    );

    // ---- ⑥ 文本层接线（本机不编译 Slint 的那一半，读源码取证） ----
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
        sidebar.contains("in property <[string]> items: [];"),
        "条目数组的**默认值必须是空** —— 内联演示数据就是从这一行开始的"
    );
    assert!(
        !sidebar.contains("SFZ · 323"),
        "⛔ 假数字（323 款采样预置）不许回到 `.slint`"
    );
    assert!(
        sidebar.contains("for item_text[item_index] in root.items : Rectangle")
            && sidebar.contains("accessible-label: item_text;")
            && sidebar.contains("accessible-item-count: root.items.length;"),
        "条目必须由 `items` 数组**实例化**（⛔ 不是 `for item_index in 8`）"
    );
    assert!(
        sidebar.contains("in property <int> active-item: 0;")
            && sidebar.contains("accessible-item-selected: item_index == root.active-item;")
            && sidebar.contains("background: item_index == root.active-item ?"),
        "选中态必须唯一读 `active-item`"
    );
    assert!(
        sidebar.contains("accessible-id: \"sidebar-empty\";")
            && sidebar.contains("visible: !root.compact && root.items.length == 0;"),
        "空态必须可寻址，且在场性 = `!compact && 条数 == 0`"
    );
    let app = std::fs::read_to_string(manifest_dir.join("ui/app.slint")).expect("读 app.slint");
    assert!(
        app.contains("callback select-sidebar-item(int);")
            && app.contains("active-item: root.sidebar-item;")
            && app.contains("root.select-sidebar-item(item);"),
        "主窗口必须把条目点击原样转给宿主，并把属性下发给资源栏"
    );
    assert!(
        app.contains("items: root.sidebar-items;")
            && app.contains("empty-reason: root.sidebar-empty-reason;"),
        "主窗口必须把宿主注入的条目数组与空态文案**转发**给资源栏"
    );
    let host_src = std::fs::read_to_string(manifest_dir.join("src/host.rs")).expect("读 host.rs");
    assert!(
        host_src.contains("ui.on_select_sidebar_item(")
            && host_src.contains("sidebar_category_content")
            && !host_src.contains("SIDEBAR_ITEM_COUNT"),
        "宿主必须是唯一的钳位 / 写属性处，且钳位读**投影条数**（⛔ 不是常量）"
    );

    report_line(&format!(
        "[sidebar-item] 分类 0 = {} 条（清单顺序 {:?}）；点击第 2 条后属性 0 → 1、\
         选中背景 {:?} / 未选中 {:?}；越界 99 → {}；分类 1 = {} 条；分类 2 = 0 条 + \
         理由「{}」",
        content.len(),
        content.items,
        before[0],
        before[1],
        content.len() - 1,
        assets.len(),
        preset.empty_reason
    ));
}
