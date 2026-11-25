//! D-UI-18 的 **B 层**：`Cmd/Ctrl+F` 在**真实窗口**上把键盘焦点交给左栏搜索框。
//!
//! ## 补的是哪一格
//!
//! 搜索框自 D-UI-14 起是**真** `TextInput`（`ui/left_panel.slint` 的
//! `browser_search_input`），鼠标点进去能敲。但**纯键盘**用户进不去：`Tab` 在画布上被解析
//! 成 `Action::ToggleView`（`cli.rs` 的快捷键表把这一条如实写成"Tab → 视图切换
//! (仅画布聚焦)"），而画布那个键盘入口 `accept` 之后 Slint 不再做焦点轮转 ⇒ 文本框在键盘
//! 上是一座**孤岛**。实测登记：`docs/ledger/app-zimmer-ui-notes.md` §31。
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
//! | B 活窗口（本文件） | 装**生产同一个** `host::wire_keys` 的真窗口 + Slint 焦点读数 | 认出来之后**真有人接**：焦点确实搬家、收起态会先展开、框不在场时如实不消费 |
//!
//! ## 判据（一条链；一个活窗口测全部，`set_platform` 一线程一次 ⇒ 一个文件一个活测试）
//!
//! ① 起点：焦点在画布键盘层，搜索框没焦点；
//! ② `Cmd/Ctrl+F` ⇒ 消费（返回值 `true`）⇒ 焦点**搬家**到搜索框，且画布层不再同时持有；
//!    搜索框在浏览器页签，取焦时页签一并切过去（`left-panel-tab` = `"browser"`）；
//! ③ 未点名的修饰变体（`Shift+F` / `Alt+F` / 裸 `f`）⇒ ⛔ 不消费、焦点不动；
//! ④ 收起态：`show-left-inspector = false` 时按同一键 ⇒
//!    先展开（左栏重现）再取焦，两个读数同时兑现；
//! ⑤ 文本层接线（源码层）：取焦函数与焦点读数各**恰好一处** —— 证明没有第二份"我以为
//!    焦点在哪"的状态。计数口径 = **代码行**（`//` 开头的行先删，见 `code_only`）：
//!    注释里提到方法名不是调用点，把它算进去会假红（实测于 run `41516839490`），
//!    ⛔ 而"为喂判据删注释"是把解释删掉、把缺陷留下。
//! ⑥ 真平台按键落进**已由键盘取得焦点**的框里：敲 `b` ⇒ 框里是 `b`，而 ⛔ 画笔工具
//!    没被切换（`active-tool` 一位不动）。这一条是"能聚焦"与"聚焦了有用"的分界。
//! ⑦ **不在场的框不许接焦点**：左栏收起时**直接**调 `focus-sidebar-search()`（绕开宿主
//!    臂的先展开）⇒ 如实**不消费**、焦点 ⛔ 不落在不在场的框上、画布层仍持有键盘。
//!    （⛔ 不许把界面按聋。）
//!
//! ## 与旧版判据的三处更正（如实登记）
//!
//! * ④ 的"折叠态"旧版写的是 `sidebar-collapsed`：那个属性在 `.slint` 侧**没有读者**
//!   （宿主写、界面不读，T-FIX-131 甄别），写它并不能真的展开左栏；真正管显隐的是
//!   `show-left-inspector`（T-FIX-131 起左栏实例以 `visible:` 门控于它）。④ 改为
//!   只置 `show-left-inspector`、断言两个读数（左栏重现 + 焦点落地）。
//!   （T-FIX-401：`sidebar-collapsed` 属性与 `Action::ToggleSidebar` 已随废弃的折叠功能删除，
//!   ④ 里那一半置位与写回断言随之删除 —— 折叠态本身从来不是左栏显隐的真相。）
//! * ⑦ 的"compact 断点"旧版写的是 `sidebar.slint` 的 `if (!root.compact)`：那个组件
//!   今天**没有实例进树**（T-FIX-131 甄别；T-FIX-218 起该组件已随 `ui/sidebar.slint`
//!   删除），`compact` 这道闸在活树里不存在。⑦ 改为
//!   活树里真实的"不在场"—— `show-left-inspector = false`，且在 Slint 层直接调函数
//!   （绕开宿主臂的先展开），断言左栏**仍收起**且焦点不动。⛔ 本条证的是"承诺写在自
//!   己的代码里"（`focus-search()` 先看 `root.visible`），与 ⑤ 的"焦点路由恰好一处"
//!   同一口径；上游对"给 `visible: false` 的控件调 `.focus()` 会怎样"没有承诺，
//!   本判据不替上游担保 —— 守门的是本仓自己的 `visible` 检查。
//! * ②④ 走 `dispatch_key_chord`（带修饰位的唯一入口）：它调的是 `invoke_key_action`
//!   —— 与用户真键盘**同一个**回调、同一张策略表、同一个 `UndoPort`，但它**绕过平台
//!   焦点路由**（不经过 `key-handler` 的 `key-pressed`）。因此 ②④ 证的是"解析 + 取焦
//!   搬家"，平台路由那一半由 ③ 的裸 `f`（真 `WindowEvent::KeyPressed`，经
//!   `key-handler` → `trigger-shortcut` → 同一个 `key-action`）覆盖。`Shift+Tab` 这
//!   类平台带不了修饰位的组合不在本文件里（见 `D-UI-19` 的同款边界）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | `apply_action` 的臂返回 `true` 而⛔ 不调用 `invoke_focus_sidebar_search` | ② 红（焦点读数不动） |
//! | 臂改成"无条件返回 `true`"（不看读数） | ⑦ 红（谎报消费） |
//! | 删掉 `left_panel.slint` 的 `public function focus-search` | 编译红（`app.slint` 的直接调用调不到） |
//! | 把 `resolve_logical` 第 3 步的 `'f'` 那一格删掉 | A 层红 ＋ 本文件 ② 红（键不消费） |
//! | 把 `cli.rs` 那一行的 `key` 写成 `PhysicalKey::KeyB` | `tests/cli_contract.rs` 的 B11b 红（标签与键位对不上） |
//! | `wire_input` 之外再写一份"焦点在搜索框"的布尔状态 | ⑤ 红（取焦函数 / 镜像绑定出现次数 != 恰好一处） |
//! | 删掉 `focus-search()` 里的 `root.visible` 检查 | ⑦ 红（谎报消费 ＋ 画布层失焦、界面变聋） |
//! | 删掉宿主臂的 `show-left-inspector` 先展开 | ④ 红（左栏不重现、焦点落不下去） |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_ui_test_port::port::{ChordKey, KeyChord, KeyChordModifiers, KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// ⑤ 的口径：只在**代码行**上数，注释里提到这个名字不算调用点。
///
/// ⚠ 这条口径本身是被 CI 教出来的：run `41516839490` 红在这里，因为 `host.rs` 的
/// 说明注释里写了一次 `ui.invoke_focus_sidebar_search()`（带括号，看起来像调用），
/// 于是"恰好一处"数成了两处。⛔ 不去删注释来喂判据（那会把要解释的东西删掉），
/// 而是把"数的是代码"这件事写进判据。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

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

fn ctrl_f() -> KeyChord {
    KeyChord::new(
        ChordKey::Character('f'),
        KeyChordModifiers {
            ctrl: true,
            ..KeyChordModifiers::default()
        },
    )
}

/// ①③②④⑥⑦ —— 搜索框取焦整条链。
///
/// 顺序有讲究：③ 的裸 `f` 必须在焦点还在画布层时按（否则 `f` 会落进搜索框当文本）；
/// ⑥ 的 `b` 必须在焦点已进框之后按（否则 `b` 会切画笔工具）。
#[test]
fn cmd_ctrl_f_moves_the_keyboard_focus_into_the_search_box() {
    let mut live = build();
    let ui = live.ui().clone_strong();

    // ① 起点：焦点在画布键盘层，搜索框没焦点，工具 1。
    assert!(canvas_layer_focused(&live), "① 起点：快捷键层必须持有键盘");
    assert!(!search_focused(&live), "① 起点：搜索框必须没焦点");
    assert_eq!(ui.get_active_tool(), 1, "① 起点：工具必须是 1");
    report_line("search-focus start: shortcut-focused=true search-focused=false tool=1");

    // ③ 未点名的变体 ⇒ 不消费、焦点不动。先走 chord 变体（Shift/Alt + F），
    // 再走真平台裸 `f`（经 key-handler → trigger-shortcut → 同一个 key-action）。
    let shift_f = live
        .dispatch_key_chord(&KeyChord::new(
            ChordKey::Character('f'),
            KeyChordModifiers {
                shift: true,
                ..KeyChordModifiers::default()
            },
        ))
        .expect("注入 Shift+F");
    assert!(!shift_f, "③ Shift+F 必须不消费");
    let alt_f = live
        .dispatch_key_chord(&KeyChord::new(
            ChordKey::Character('f'),
            KeyChordModifiers {
                alt: true,
                ..KeyChordModifiers::default()
            },
        ))
        .expect("注入 Alt+F");
    assert!(!alt_f, "③ Alt+F 必须不消费");
    live.dispatch_key_press(KeyCode::Character('f'))
        .expect("注入裸 f");
    assert!(canvas_layer_focused(&live), "③ 裸 f 之后焦点必须仍在画布层");
    assert!(!search_focused(&live), "③ 裸 f 必须不把焦点搬进搜索框");
    assert_eq!(ui.get_sidebar_search(), "", "③ 裸 f 不能变成搜索框里的文本");
    report_line("search-focus variants-refused: shift/alt/bare-f unconsumed");

    // ② Cmd/Ctrl+F ⇒ 消费、焦点搬家、页签切到浏览器。
    let consumed = live.dispatch_key_chord(&ctrl_f()).expect("注入 Cmd/Ctrl+F");
    assert!(consumed, "② Cmd/Ctrl+F 必须消费");
    assert!(search_focused(&live), "② 焦点必须搬进搜索框");
    assert!(!canvas_layer_focused(&live), "② 画布层必须不再同时持有键盘");
    assert_eq!(
        ui.get_left_panel_tab(),
        "browser",
        "② 取焦时页签必须切到浏览器"
    );
    report_line("search-focus moved: search-focused=true tab=browser");

    // ④ 收起态：左栏收起 ⇒ 同一键先展开再取焦。
    ui.set_show_left_inspector(false);
    let consumed = live
        .dispatch_key_chord(&ctrl_f())
        .expect("收起态注入 Cmd/Ctrl+F");
    assert!(consumed, "④ 收起态 Cmd/Ctrl+F 必须消费");
    assert!(
        ui.get_show_left_inspector(),
        "④ 左栏必须先展开（show-left-inspector=true）"
    );
    assert!(search_focused(&live), "④ 展开后焦点必须落在搜索框");
    report_line("search-focus collapsed-expanded: panel-shown=true");

    // ⑥ 真平台按键落进已取焦的框里：`b` 变成文本，画笔工具不动。
    live.dispatch_key_press(KeyCode::Character('b'))
        .expect("框内注入 b");
    assert_eq!(ui.get_sidebar_search(), "b", "⑥ 框里必须是 b");
    assert_eq!(
        ui.get_active_tool(),
        1,
        "⑥ b 不能切画笔工具（active-tool 仍是 1）"
    );
    report_line("search-focus typed: search=b tool=1");

    // ⑦ 不在场的框不许接焦点：先把焦点送回画布层、收起左栏，再**直接**调取焦函数
    // （绕开宿主臂的先展开）⇒ 不消费、不谎报、左栏仍收起。
    ui.invoke_focus_shortcut_layer();
    assert!(canvas_layer_focused(&live), "⑦ 前置：焦点必须先回到画布层");
    ui.set_show_left_inspector(false);
    ui.invoke_focus_sidebar_search();
    assert!(!search_focused(&live), "⑦ 不在场的框必须不接焦点");
    assert!(
        canvas_layer_focused(&live),
        "⑦ 画布层必须仍持有键盘（界面不能变聋）"
    );
    assert!(!ui.get_show_left_inspector(), "⑦ 直接调用不能偷偷展开左栏");
    report_line("search-focus absent-refused: hidden panel keeps focus on canvas");
}

/// ⑤ —— 取焦函数与焦点读数各恰好一处，没有第二份焦点状态。
#[test]
fn the_focus_route_has_exactly_one_implementation() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    let panel = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/left_panel.slint"),
    )
    .expect("读 left_panel.slint");

    let code_app = code_only(&app);
    let code_host = code_only(&host);
    let code_panel = code_only(&panel);

    // 取焦函数的唯一实现在左栏；MainWindow 的函数直接调它（T-FIX-131 起主工作区行
    // 以 `visible:` 门控，id 够得到 —— ⛔ 不走 `changed` 脉冲：实测绑定惰性求值，
    // 处理器在无头侧不触发、在生产侧也要等下一拍，消费返回值会谎报）。
    assert_eq!(
        code_panel.matches("public function focus-search()").count(),
        1,
        "left_panel.slint 里 focus-search 必须恰好一处"
    );
    assert_eq!(
        code_app.matches("left-panel.focus-search()").count(),
        1,
        "app.slint 里 focus-sidebar-search 必须恰好一次直接调 left-panel.focus-search()"
    );
    // 焦点读数：MainWindow 里恰好一条镜像绑定（从左栏的 search-has-focus 推导），
    // 全仓没有任何一处写 sidebar-search-focused（它是推导读数，不是状态）。
    assert_eq!(
        code_app
            .matches("sidebar-search-focused: left-panel.search-has-focus")
            .count(),
        1,
        "app.slint 里 sidebar-search-focused 的镜像绑定必须恰好一条"
    );
    // 宿主臂恰好一次调取焦函数、一次读焦点读数。
    assert_eq!(
        code_host
            .matches("ui.invoke_focus_sidebar_search()")
            .count(),
        1,
        "host.rs 里 invoke_focus_sidebar_search 必须恰好一处（FocusSidebarSearch 臂）"
    );
    assert_eq!(
        code_host.matches("ui.get_sidebar_search_focused()").count(),
        1,
        "host.rs 里 sidebar-search-focused 的读数必须恰好一处（同一臂的返回值）"
    );
}
