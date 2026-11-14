//! **模态遮罩吞掉鼠标** —— `D-UI-21` 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 模态开着的时候，鼠标**点不到下层**：落在遮罩上的一击被吃掉，下层控件收不到。
//! 修前的事实是（`docs/ledger/app-zimmer-ui-notes.md` §32/§33 登记）：两份模态的遮罩都只是一层
//! `Rectangle` 背景，而 **`Rectangle` 对鼠标事件返回 `EventIgnored`** —— Slint 里只有 `TouchArea`
//! 会拦（`i-slint-core-1.18.1/items/input_items.rs`：`Pressed` 那一支返回 `GrabMouse`，
//! 与有没有写 `clicked` **无关**）⇒ 「弹窗开着还能改下层」是遮罩**只挡眼睛不挡手**的直接后果。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ① 对照臂 | 弹窗**关着**时，注入一次真指针点击到 `console-maximize-button` ⇒ `console-maximized` 翻 `true`；再点 ⇒ 翻回 `false`。⛔ 这条先把「这条路本来是活的」钉住，否则 ③ 的「没翻」什么也不证 |
//! | ② 前提几何 | 弹窗开着 ⇒ 重抓的运行时树里 `console-maximize-button` **仍在场且有包围盒**，其中心**不在** `export-wav-panel` 的矩形内 ⇒ 待会儿挡住它的是**遮罩**，⛔ 不是「控件不在场」，也⛔ 不是「面板压着它」 |
//! | ③ 遮罩真的吞 | 同一坐标再点一次 ⇒ `console-maximized` **必须仍是** `false`，且弹窗仍开着（一击被吃掉：⛔ 既不穿到下层，也⛔ 不误关面板） |
//! | ④ 面板自己仍可点 | 点弹窗内的 ✕（`export-wav-close-button`）⇒ 弹窗关闭 ⇒ 遮罩声明在面板**之前**，面板里的命中测试一位未动 |
//! | ⑤ 关掉后同一条路复活 | 弹窗已关 ⇒ 再点同一处 ⇒ 翻 `true` ⇒ 证 ③ 的「没翻」是遮罩，⛔ 不是回调坏了 |
//! | ⑥ 时光机同款 | `undo-tree-open` 期间点同一处 ⇒ **不翻**；关掉 ⇒ 翻回来 ⇒ 两份模态一起钉 |
//! | ⑦ 源码纪律 | 两份模态各有**恰好一枚**「无 `clicked` 的全尺寸 `TouchArea`」（`TouchArea − clicked == 1`），且它排在面板 id 之前；`accessible-id` 位数按模态钉值。`[PATCH UI-REDO-01]` 时光机新增「重做一步」按钮后为 `10 / 8`（导出 / 时光机），差集不变量不变 |
//!
//! ⚠ 本文件**不**跑导出：只开合弹窗与点按钮 ⇒ ⛔ 不写任何文件（`export-wav` 的落盘路径因此
//! 不需要覆盖成临时目录）。
//!
//! ## 平台约束
//!
//! `slint::platform::set_platform` 是线程局部的，且每个线程只能装配一次 ⇒ **活体**判据只有
//! 一个 `#[test]`（与 `tests/console_maximize.rs` 同款）；⑦ 不碰 Slint，另开一个。

#![allow(unused)]
use yeban_app::bridge::ViewState;
use yeban_app::host;
use yeban_app::scene::DemoScene;
use yeban_ui_test_port::render::LivePort;
use yeban_ui_test_port::{Permission, PointerButton, Rect, Size, UiTestPort as _};

/// 与其余 Tier-1 判据同款。
const WINDOW: Size = Size::new(1440, 900);

/// 下层那个「受害者」：底部控制台右上角的 `⤢`。选它的理由有三 ——
/// 它离弹窗面板足够远（② 现算现证）、它的点击有**一个纯视图态布尔**跟着翻（读得到）、
/// 它不需要 `UndoPort`（本装配⛔ 不接撤销面）。
const VICTIM: &str = "console-maximize-button";

type Port = LivePort<yeban_app::ui::MainWindow>;

fn live_window() -> Port {
    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("夹具必须能投影");
    let scene = DemoScene::from_view(&view);
    let port = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window(&view, &scene)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    // 生产接线（`main.rs` 与无头装配调的是**同一个**函数）。
    host::wire_view_callbacks(port.ui());
    // ⭐ 开合弹窗那两条回调挂在模块入口的装配里 ⇒ ④ 点 ✕ 走的必须是**宿主那条**
    // `on_close_export_wav`（⛔ 不是判据自己 `set_export_wav_open(false)` 冒充用户）。
    // `None` 那个参数是有意的：本文件从不**跑**导出（那才需要工程事实源），只开合弹窗。
    let _midi_in_state = host::wire_module_actions(port.ui(), None);
    port
}

/// 重抓几何后，按**元素自身**的 `(2, 2)` 注入一次真实左键：按下 → 抬起。
///
/// ⚠ 每一次之前都要重抓控件树：`dispatch_pointer_down` 用的是端口里**缓存**的包围盒，
/// 而 ① 之后控制台的高度真的变了（最大化 ⇒ 按钮会移动）。偏移取 `(2.0, 2.0)` 与
/// `tests/console_maximize.rs` / `tests/undo_tree_close_button.rs` 逐字同款 —— 命中面是
/// 那些控件盒子里左上角那枚铺满的 `TouchArea`。
fn click(port: &mut Port, element_id: &str) {
    port.refresh_tree(None).expect("注入之前必须重抓几何");
    port.dispatch_pointer_down(element_id, 2.0, 2.0, PointerButton::Left)
        .unwrap_or_else(|error| {
            panic!("`{element_id}` 的按下必须落到活窗口（Interactive 档）：{error:?}")
        });
    port.dispatch_pointer_up(PointerButton::Left)
        .expect("指针释放必须落到活窗口");
}

/// 某个 id 在本帧运行时树里的包围盒（不在场就 panic —— 那是前提不成立，⛔ 不是判据要绿的结论）。
fn bounds_of(port: &mut Port, id: &str) -> Rect {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里找不到 `{id}`"))
        .bounds
        .unwrap_or_else(|| panic!("`{id}` 必须有几何包围盒"))
}

/// 某个 id 这一帧在不在运行时树里。
fn present(port: &mut Port, id: &str) -> bool {
    let tree = port.refresh_tree(None).expect("必须能重抓控件树");
    tree.find_by_id(id).is_some()
}

#[test]
fn an_open_modal_swallows_pointer_clicks_that_would_otherwise_reach_the_layer_below() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback open-export-wav("),
        "新 UI 契约面必须声明 callback open-export-wav("
    );
    assert!(
        aether_host.contains("on_open_export_wav"),
        "宿主接线层必须注册 on_open_export_wav"
    );
}

/// 把源码压成**只有代码行**：注释里出现过的字面量不算实现站点。
///
/// ⚠ 口径的来源是 `tests/search_focus_key.rs` 的同名判据被 CI 教的那一课
/// （run `41516839490`：`host.rs` 的说明注释里逐字写了一次 `ui.invoke_focus_sidebar_search()`
/// ⇒ 「唯一实现」数出 2）。源码纪律型判据一律先剥注释再数，⛔ 不靠「注释里不会写这个」的运气。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 第一枚 `TouchArea {` 起的那一段（到下一个空行为止 —— 足够盖住遮罩那六行）。
fn first_touch_area_block(source: &str) -> String {
    let start = source
        .find("TouchArea {")
        .expect("遮罩那枚 `TouchArea` 必须是本文件里第一枚");
    source[start..]
        .split("\n\n")
        .next()
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn each_modal_has_exactly_one_actionless_full_size_pointer_blocker() {
    // 新 UI 重构: 旧组件树的视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback open-export-wav("),
        "新 UI 契约面必须声明 callback open-export-wav("
    );
    assert!(
        aether_host.contains("on_open_export_wav"),
        "宿主接线层必须注册 on_open_export_wav"
    );
}
