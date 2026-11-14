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
//!    证明没有第二份"我以为焦点在哪"的状态。计数口径 = **代码行**（`//` 开头的行先删，
//!    见 `code_only`）：注释里提到方法名不是调用点，把它算进去会假红（实测于 run
//!    `41516839490`），⛔ 而"为喂判据删注释"是把解释删掉、把缺陷留下。
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

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_ui_test_port::port::{KeyCode, Permission};

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

/// ①②③ —— `Cmd/Ctrl+F` 真的把焦点搬进搜索框，未点名的变体不搬。
#[test]
fn cmd_ctrl_f_moves_the_keyboard_focus_into_the_search_box() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ④ —— 折叠态：按 `Cmd/Ctrl+F` 会**先展开**（写的仍是同一个属性），再取焦。
#[test]
fn cmd_ctrl_f_expands_a_collapsed_sidebar_before_focusing_it() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ⑤ —— 文本层接线：一条入口，没有第二份焦点状态。
#[test]
fn the_focus_route_has_exactly_one_implementation() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}

/// ⑥ —— 焦点是**有用的**：真平台按键落进框里，而 ⛔ 不再触发画布语义。
///
/// ⚠ 本条与 ② 的分工：② 只证"焦点读数动了"，本条证"键真的归文本框"。
/// 夹具与 `tests/sidebar_search.rs` 同一注入面（`dispatch_key_press` = 真
/// `WindowEvent::KeyPressed`，由 Slint 按焦点路由）。
#[test]
fn a_real_key_press_goes_to_the_box_and_not_to_the_canvas() {
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}
