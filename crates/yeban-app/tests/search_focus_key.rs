//! D-UI-18 的 **B 层**：`Cmd/Ctrl+F` 在**真实窗口**上把键盘焦点交给左栏搜索框。
//!
//! ## 补的是哪一格
//!
//! 搜索框自 D-UI-14 起是**真** `TextInput`（`ui/sidebar.slint` 的 `sidebar-search-input`），
//! 鼠标点进去能敲。但**纯键盘**用户进不去：`Tab` 在画布上被解析成 `Action::ToggleView`
//! （`cli.rs` 的快捷键表把这一条如实写成"Tab → 视图切换 (仅画布聚焦)"），而画布那个
//! 键盘入口 `accept` 之后 Slint 不再做焦点轮转 ⇒ 文本框在键盘上是一座**孤岛**。
//! 实测登记：`docs/ledger/app-zimmer-ui-notes.md` §31。
//!
//! 本切片**只**接"有键能进框"这一半。`[UI-A11Y-003]` §7.3 第 1 条要的是"严密的 Tab 序列"
//! （`[TEST-SPEC-005]` 用例 ② 要断言焦点顺序），而那要先改 `Tab` 的既有语义 ⇒ 属人类裁决，
//! 已登记，⛔ 不在本文件里顺手发明。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `cmd_ctrl_f_focuses_the_sidebar_search_and_stay_behind_the_text_policy` | 这一个键位被状态机**认成**这一个动作；文本域 / 合成态策略仍排在它前面 |
//! | B 活窗口（本文件） | 装**生产同一个** `host::wire_keys` 的真窗口 + Slint 焦点读数 | 认出来之后**真有人接**：焦点确实搬家、折叠态会先展开、框不在场时如实不消费 |
//!
//! ## 判据
//!
//! ① 起点：焦点在画布键盘层，搜索框没焦点；
//! ② `Cmd/Ctrl+F` ⇒ 消费（返回值 `true`）⇒ 焦点**搬家**到搜索框，且画布层不再同时持有；
//! ③ 未点名的修饰变体（`Shift+F` / `Alt+F` / 裸 `f`）⇒ ⛔ 不消费、焦点不动；
//! ④ 折叠态：`sidebar-collapsed = true` 时按同一键 ⇒ 先展开（写的仍是同一个属性）再取焦，
//!    两个读数同时兑现；
//! ⑤ 文本层接线（源码层，与 `tests/sidebar_search.rs` ⑨ 同一口径）：`public function`
//!    在 `sidebar.slint` / `app.slint` 各**恰好一处**，宿主臂**恰好一次**调用它 ——
//!    证明没有第二份"我以为焦点在哪"的状态；
//! ⑥ 真平台按键落进**已由键盘取得焦点**的框里：敲 `b` ⇒ 框里是 `b`，而 ⛔ 画笔工具
//!    没被切换（`active-tool` 一位不动）。这一条是"能聚焦"与"聚焦了有用"的分界。
//! ⑦ `compact` 断点（框**离场**）⇒ 这一键如实**不消费**、焦点⛔ 不落在不在场的框上、
//!    画布层仍持有键盘。这道闸由 `sidebar.slint` 的 `if (!root.compact)` 落下，
//!    ⛔ 不是"信 Slint 会替我拦"（上游对"给 `visible: false` 的控件调 `.focus()`"无承诺）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | `apply_action` 的臂返回 `true` 而⛔ 不调用 `invoke_focus_sidebar_search` | ② 红（焦点读数不动） |
//! | 臂改成"无条件返回 `true`"（不看读数） | ④ 的窄栏分支无法证伪 ⇒ 但 ⑥ 之前先由"折叠态"覆盖；⛔ 本表如实说明判别力落在哪一步 |
//! | 删掉 `sidebar.slint` 的 `public function` | 编译红（`app.slint` 调不到） |
//! | 把 `resolve_logical` 第 3 步的 `'f'` 那一格删掉 | A 层红 ＋ 本文件 ② 红（键不消费） |
//! | 把 `cli.rs` 那一行的 `key` 写成 `PhysicalKey::KeyB` | `tests/cli_contract.rs` 的 B11b 红（标签与键位对不上） |
//! | `wire_input` 之外再写一份"焦点在搜索框"的布尔状态 | ⑤ 红（`public function` 出现次数 != 恰好一处） |
//! | 删掉 `sidebar.slint` 里 `if (!root.compact)` 这道闸 | 判别力**取决于上游**：若上游肯给 `visible: false` 的控件置焦 ⇒ ⑦ 红（谎报消费 ＋ 画布层失焦、界面变聋）；若上游本就拒绝 ⇒ ⑦ 仍绿。⚠ 本行如实登记这个边界：⑦ 钉的是"承诺写在本仓的代码里"，不是"上游会兜住" |

#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_ui_test_port::port::{KeyCode, Permission};

/// 焦点读数：`app.slint` 镜像出来的两条（`sidebar-search-focused` / `shortcut-layer-focused`）。
fn search_focused(ui: &live::LiveUi) -> bool {
    ui.ui().get_sidebar_search_focused()
}

fn canvas_layer_focused(ui: &live::LiveUi) -> bool {
    ui.ui().get_shortcut_layer_focused()
}

fn build() -> live::LiveUi {
    let project = yeban_model::samples::filled_project();
    build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面")
}

/// ①②③ —— `Cmd/Ctrl+F` 真的把焦点搬进搜索框，未点名的变体不搬。
#[test]
fn cmd_ctrl_f_moves_the_keyboard_focus_into_the_search_box() {
    let ui = build();

    // ---- ① 起点 ----
    assert!(
        canvas_layer_focused(&ui),
        "① 起点：键盘归画布那层（`forward-focus: key-handler` 的机械证据）"
    );
    assert!(
        !search_focused(&ui),
        "① 起点：搜索框不该有焦点（默认帧一位不变的前提）"
    );

    // ---- ② 按 `Cmd/Ctrl+F` ----
    let consumed = ui
        .ui()
        .invoke_key_action("f".into(), false, true, false, false);
    assert!(
        consumed,
        "② 宿主必须**消费**这一键（返回值 = 界面是否消费，与 `D-UI-11` 同一口径）"
    );
    assert!(
        search_focused(&ui),
        "② 按完之后焦点必须落在搜索框上（读的是 `sidebar-search-input.has-focus` 本身）"
    );
    assert!(
        !canvas_layer_focused(&ui),
        "② 焦点是**唯一**的：画布层不得同时持有"
    );

    // ---- ③ 未点名的修饰变体 ⇒ 不消费、焦点不动 ----
    for (shift, ctrl, alt, _meta) in [
        (true, false, false, false),  // `Shift+F`
        (false, false, true, false),  // `Alt+F`
        (false, false, false, false), // 裸 `f`
    ] {
        let consumed = ui
            .ui()
            .invoke_key_action("f".into(), shift, ctrl, alt, meta);
        assert!(
            !consumed,
            "③ `shift={shift} ctrl={ctrl} alt={alt}` 的组合未被点名 ⇒ 界面不得消费"
        );
        assert!(
            search_focused(&ui),
            "③ 未点名的组合不得把焦点**弄出去**（焦点仍应在框里）"
        );
    }
}

/// ④ —— 折叠态：按 `Cmd/Ctrl+F` 会**先展开**（写的仍是同一个属性），再取焦。
#[test]
fn cmd_ctrl_f_expands_a_collapsed_sidebar_before_focusing_it() {
    let ui = build();
    // 折叠的唯一写者是 `wire_view_callbacks` 的回调与 `Action::ToggleSidebar` 那条臂；
    // 这里直接写同一个属性来造前置态（⛔ 不是第四个写者，判据造的是**状态**不是**实现**）。
    ui.ui().set_sidebar_collapsed(true);
    assert!(!search_focused(&ui), "④ 前置：折叠时框不在场 ⇒ 不该有焦点");

    let consumed = ui
        .ui()
        .invoke_key_action("f".into(), false, true, false, false);

    assert!(
        consumed,
        "④ 折叠态按 `Cmd/Ctrl+F` 仍应消费（展开 + 取焦都兑现了）"
    );
    assert!(
        !ui.ui().get_sidebar_collapsed(),
        "④ 折叠必须被展开 —— 写的必须是 `sidebar-collapsed` 那**同一个**属性"
    );
    assert!(
        search_focused(&ui),
        "④ 展开之后焦点必须真的落在框上（⛔ 不是「展开了就算成功」）"
    );
}

/// ⑤ —— 文本层接线：一条入口，没有第二份焦点状态。
#[test]
fn the_focus_route_has_exactly_one_implementation() {
    let sidebar = include_str!("../ui/sidebar.slint");
    let app = include_str!("../ui/app.slint");
    let host = include_str!("../src/host.rs");

    assert_eq!(
        sidebar.matches("public function focus-search()").count(),
        1,
        "⑤ `Sidebar` 暴露的取焦入口必须**恰好一个**"
    );
    assert_eq!(
        sidebar.matches("sidebar-search-input.focus()").count(),
        1,
        "⑤ `focus()` 只许打在 `sidebar-search-input` 这一个控件上"
    );
    assert_eq!(
        app.matches("sidebar.focus-search()").count(),
        1,
        "⑤ `MainWindow` 的转发必须**恰好一次**（⛔ 不在界面层再判一次焦点）"
    );
    assert_eq!(
        host.matches("invoke_focus_sidebar_search()").count(),
        1,
        "⑤ 宿主调用点必须**恰好一处**（`apply_action` 的 `FocusSidebarSearch` 臂）"
    );
    // ⭐ 焦点不是属性：宿主⛔ 不许写一个"我以为焦点在哪"的布尔。
    assert!(
        !host.contains("set_sidebar_search_focused("),
        "⑤ `sidebar-search-focused` 是 `out` 读数，出现 setter = 造出第二份焦点状态"
    );
}

/// ⑥ —— 焦点是**有用的**：真平台按键落进框里，而 ⛔ 不再触发画布语义。
///
/// ⚠ 本条与 ② 的分工：② 只证"焦点读数动了"，本条证"键真的归文本框"。
/// 夹具与 `tests/sidebar_search.rs` 同一注入面（`dispatch_key_press` = 真
/// `WindowEvent::KeyPressed`，由 Slint 按焦点路由）。
#[test]
fn a_real_key_press_goes_to_the_box_and_not_to_the_canvas() {
    let mut ui = build();
    // 先用键盘拿到焦点（本切片的入口），⛔ 不点鼠标 —— 点的就不是"键盘可达"。
    assert!(
        ui.ui()
            .invoke_key_action("f".into(), false, true, false, false),
        "⑥ 前置：`Cmd/Ctrl+F` 必须消费"
    );
    let tool_before = ui.ui().get_active_tool();

    ui.dispatch_key_press(KeyCode::Character('b'))
        .expect("⑥ `b` 必须落到真实窗口（Interactive 档）");

    assert_eq!(
        ui.ui().get_sidebar_search().to_string(),
        "b",
        "⑥ 敲进去的字必须进框（焦点真的归文本框，不是只改了个读数）"
    );
    assert_eq!(
        ui.ui().get_active_tool(),
        tool_before,
        "⑥ 同一个 `b` ⛔ 不得再触发画布上的单键语义（`[UI-A11Y-002]` 要的就是这一条）"
    );
}

/// ⑦ —— **不在场的框不许接焦点**：`compact` 断点下搜索节点离场，这一键必须如实不消费，
/// 而画布层必须仍然持有键盘（⛔ 不许把界面按聋）。
///
/// 本条钉的是 `ui/sidebar.slint` 里那句 `if (!root.compact)` —— 这道闸由**本仓**落下，
/// ⛔ 不是"信 Slint 会替我拦"：上游对"给 `visible: false` 的控件调 `.focus()` 会怎样"
/// 没有承诺，本机也没实测。若上游本来就会拒绝，删掉这道闸 ⑦ 仍绿；本条证的是
/// **"承诺写在自己的代码里"**，与 ⑤ 的"焦点路由恰好一处"同一口径。
#[test]
fn an_absent_search_box_refuses_the_key_instead_of_making_the_ui_go_deaf() {
    let ui = build();
    // 造的是**状态**而不是实现：`compact` 是 `MainWindow` 的 `in-out` 属性（绑定
    // `self.width < Tokens.breakpoint-full-hd`），置位手法取自
    // `src/test_port_adapter.rs`（`port.ui().set_compact(true)`）。⛔ 不是抄
    // `tests/compact_follows_window_width.rs` —— 那条判据明写自己「只走生产构造入口，不碰
    // `set_compact`」。
    ui.ui().set_compact(true);
    assert!(
        canvas_layer_focused(&ui),
        "⑦ 前置：焦点还在画布键盘层（`compact` 本身不该动焦点）"
    );

    let consumed = ui
        .ui()
        .invoke_key_action("f".into(), false, true, false, false);

    assert!(
        !consumed,
        "⑦ 框离场 ⇒ 焦点读数必须是 `false` ⇒ 宿主必须**不消费**（⛔ 不假装成功）"
    );
    assert!(!search_focused(&ui), "⑦ 焦点不得落在一个不在场的框上");
    assert!(
        canvas_layer_focused(&ui),
        "⑦ 画布层必须仍然持有键盘 —— 一次失败的聚焦⛔ 不能让快捷键失灵"
    );
}
