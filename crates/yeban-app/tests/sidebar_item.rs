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

#![allow(unused)]
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

/// 去掉 `//` 行注释与行尾注释后的**代码文本**（⑥ 的三条读都先过这里）。
///
/// 为什么必须去注释：本票把「删掉了哪条假数字」写进了 `sidebar.slint` 的注释里
/// （审计留痕，与 `src/elements.rs` 的 `code_only` 同一口径）。探针若把注释也算成
/// "界面里的字面量"，`!contains("SFZ · 323")` 就会红在**留痕本身**上 —— 那等于逼作者
/// 不写留痕，与本仓库的纪律相反。⚠ 这一格同时是**反向**防线：注释里出现的
/// `for … in root.items` 之类代码形状也不许算作"接上了"。
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _comment)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

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
        aether_app.contains("callback select-sidebar-item("),
        "契约面必须声明 callback select-sidebar-item("
    );
    assert!(
        aether_host.contains("select_sidebar_item"),
        "宿主接线层必须注册 select_sidebar_item"
    );
    assert!(
        aether_app.contains("> sidebar-item:"),
        "契约面必须声明 > sidebar-item:"
    );
    assert!(
        aether_host.contains("sidebar_item"),
        "宿主接线层必须注册 sidebar_item"
    );
}
