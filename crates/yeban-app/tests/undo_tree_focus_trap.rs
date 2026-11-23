//! `[UI-A11Y-003]` §7.3 / `[TEST-SPEC-005]` 用例 ③：模态**焦点闭环**与关闭后**焦点复位**。
//!
//! ## 为什么这里能按"焦点值"断言（既有 a11y 判据做不到）
//!
//! 控件树里没有焦点字段（`yeban_ui_test_port::tree` 与 `yeban_ui_mcp::tree` 里 `focus` 命中 0），
//! 因此 `tests/a11y_screen_reader.rs` 只能按"寻址面"近似。本切片在 `ui/app.slint` 里把两个
//! `FocusScope.has-focus` **镜像**成 MainWindow 的只读属性：
//!
//! * `undo-tree-focus-trapped` —— 模态自己的 `FocusScope`（`undo_tree_trap`）是否持有焦点；
//! * `shortcut-layer-focused` —— 唯一键盘事件源 `key-handler` 是否持有焦点。
//!
//! 两个属性只是 Slint 焦点状态的**读数**：界面自己不读它们 ⇒ 不引入第二份状态。
//!
//! ## 判据（一条链，六个读数）
//!
//! ① 起点：快捷键层有焦点、模态未持有；
//! ② 打开（端口 + `apply_undo`，与宿主同一个开关）⇒ 焦点**真的**移进模态；
//! ③ 模态内按 `3` ⇒ **到不了**宿主（`active-tool` 一位不动）—— 见下面的"与旧版判据的
//!    一处更正"：陷阱按 `D-UI-19` 的契约 `reject` 其余键，模态持键盘时 DAW 收不到键；
//! ④ 模态内按 `Tab` ⇒ 焦点**不**轮转出去，也不触发画布语义的切视图（视图不变）；
//! ⑤ `Esc` 关闭 ⇒ 焦点复位到快捷键层，随后 `1` 又能到达宿主（`active-tool` = 1）。
//!    为让 ⑤ 的第二半有判别力：开模态前先按 `3` 把工具切到 3（起点读数），
//!    关模态后再按 `1` —— 工具必须回到 1。
//!
//! ⛔ 本判据**不**声称"模态内各控件之间的 Tab 顺序"：本骨架里模态的按钮不是 `FocusScope`
//! （只有 `TouchArea`），闭环 = 焦点停在模态这一个作用域上。
//!
//! ## 与旧版判据的一处更正（如实登记）
//!
//! 旧版文件头的 ③ 写的是"模态内按 `3` ⇒ 仍从同一个键盘入口到达宿主（`active-tool` = 3）"。
//! 那条描述的是一个**从未存在过**的设计：`undo_tree_trap` 这个 `FocusScope` 在
//! `T-FIX-131` 之前根本不存在（`app.slint` 里只有注释里提到它），而 `key-handler`
//! 不是模态的祖先 ⇒ 焦点一旦进了模态，键不可能再冒到快捷键层。其余四只模态
//! （导出 / 人性化 / 鼓机 / 音频设置）的陷阱在 `D-UI-19` 起就是"其余键一律 `reject`"——
//! 判据 `tests/export_dialog_key_capture.rs` 的 ⑥ 把这叫"捕获"。本恢复沿用同一契约：
//! ③ 断言键**进不去**，与 ⑤ 的"关后又能进"配成一对 —— 那才是"陷阱"二字的判别力所在。
//!
//! ## 本判据证明什么、不证明什么
//!
//! * 证明：打开/关闭时焦点在快捷键层与模态陷阱之间**真的搬家**（Slint 自己的 `has-focus`
//!   读数）；陷阱内 `Tab` 不逃逸、不触发画布切视图；`Esc` 经既有 `close` 回调关闭并复位；
//!   陷阱内其余键到不了 DAW（捕获）；关闭后键又能到达宿主。
//! * 不证明：模态内各控件之间的 Tab 顺序（按钮没有 `FocusScope`，无序可言）；
//!   真实键盘上的 `Shift+Tab`（平台事件通道带不了修饰位，见 `D-UI-19` 的同款边界）；
//!   像素（无头测试不截图，截图判据另有 `tests/golden/`）；
//!   其余四只模态的入口（见 `tests/modal_focus_entry.rs`）。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use slint::ComponentHandle;
use yeban_app::host;
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与既有撤销判据同一个夹具常量）。
const NOW: u64 = 1_760_000_000_000;

fn build() -> (live::LiveUi, Rc<UndoPort>) {
    let project = yeban_model::samples::filled_project();
    let undo = Rc::new(UndoPort::new(
        UndoSession::open("<undo-focus-trap>", "yeban-app", project.clone(), NOW)
            .expect("撤销会话必须能打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            undo: Some(Rc::clone(&undo)),
            ..LiveWiringOptions::default()
        },
    )
    .expect("真实界面 + Tier-1 执行面");
    (live, undo)
}

/// ①②③④⑤ —— 焦点闭环整条链。
#[test]
fn the_undo_tree_traps_focus_and_restores_it_to_the_shortcut_layer() {
    let (mut live, undo) = build();
    // 强引用句柄（与 `live_surface.rs` 的装配同款）：下面既要 `live.dispatch_key_press`
    // （`&mut`），又要读窗口属性 ⇒ 不能只借 `live.ui()`。
    let ui = live.ui().clone_strong();

    // ① 起点：快捷键层持焦、模态未持有、工具 1。
    assert!(
        ui.get_shortcut_layer_focused(),
        "起点：快捷键层必须持有键盘"
    );
    assert!(
        !ui.get_undo_tree_focus_trapped(),
        "起点：模态必须未持有焦点"
    );
    assert_eq!(ui.get_active_tool(), 1, "起点：工具必须是 1");
    report_line("undo-trap start: shortcut-focused=true trapped=false tool=1");

    // ⑤ 的第二半要判别：先把工具切到 3（键能到达宿主的证据）。
    live.dispatch_key_press(KeyCode::Character('3'))
        .expect("注入 3");
    assert_eq!(ui.get_active_tool(), 3, "起点：按 3 必须把工具切到 3");
    report_line("undo-trap tool-3: active-tool=3");

    // ② 打开（端口 + apply_undo，与宿主同一个开关）⇒ 焦点真的移进模态。
    undo.perform(UiAction::OpenUndoTree);
    host::apply_undo(&ui, &undo);
    assert!(ui.get_undo_tree_open(), "打开：undo-tree-open 必须为真");
    assert!(
        ui.get_undo_tree_focus_trapped(),
        "打开：焦点必须移进模态陷阱（undo-tree-focus-trapped=true）"
    );
    assert!(
        !ui.get_shortcut_layer_focused(),
        "打开：快捷键层必须失去焦点"
    );
    report_line("undo-trap opened: trapped=true shortcut-focused=false");

    // ③ 模态内按 `3` ⇒ 到不了宿主（捕获），工具一位不动。
    live.dispatch_key_press(KeyCode::Character('3'))
        .expect("模态内注入 3");
    assert_eq!(
        ui.get_active_tool(),
        3,
        "捕获：模态内按 3 必须到不了宿主（active-tool 仍是 3）"
    );

    // ④ 模态内按 `Tab` ⇒ 焦点不轮转出去，也不触发画布语义的切视图。
    live.dispatch_key_press(KeyCode::Tab)
        .expect("模态内注入 Tab");
    assert!(
        ui.get_undo_tree_focus_trapped(),
        "Tab 之后焦点必须仍在模态内"
    );
    assert!(ui.get_arrangement_view(), "Tab 必须不触发画布语义的切视图");
    report_line("undo-trap tab-held: trapped=true arrangement-view=true");

    // ⑤ `Esc` 关闭 ⇒ 焦点复位到快捷键层，随后 `1` 又能到达宿主。
    live.dispatch_key_press(KeyCode::Escape)
        .expect("模态内注入 Esc");
    assert!(!ui.get_undo_tree_open(), "Esc：模态必须关闭");
    assert!(
        !ui.get_undo_tree_focus_trapped(),
        "Esc：模态必须不再持有焦点"
    );
    assert!(
        ui.get_shortcut_layer_focused(),
        "Esc：焦点必须复位到快捷键层"
    );
    live.dispatch_key_press(KeyCode::Character('1'))
        .expect("关闭后注入 1");
    assert_eq!(
        ui.get_active_tool(),
        1,
        "关闭后按 1 必须又能到达宿主（active-tool=1）"
    );
    report_line("undo-trap closed: shortcut-focused=true tool=1");
}

/// ⑥ 出处纪律（⛔ 不开窗口）：焦点闭环只有一份接线。
///
/// * `invoke_focus_undo_tree_trap` 在 `host.rs` 的代码行里恰好一处（`apply_undo`），
///   关闭那一侧的 `invoke_focus_shortcut_layer` 不计入 —— 它是所有模态共用的复位点；
/// * `undo-tree-focus-trapped` 在 `app.slint` 里恰好一条绑定（从陷阱的 `has-focus` 推导），
///   全仓没有任何一处**写**它；
/// * `UndoTreeModal` 里恰好一个 `FocusScope`（陷阱本身）⇒ ⛔ 没有第二个键盘入口；
/// * 没有新增可寻址控件 ⇒ `accessible-id:` 条数一位不改（陷阱不设 `accessible-id`，
///   与其余四只模态的陷阱同款）。
#[test]
fn the_focus_trap_is_single_sourced_and_adds_no_addressable_control() {
    let app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    let modal = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/dialogs/undo_tree_modal.slint"),
    )
    .expect("读 undo_tree_modal.slint");

    let code_app = code_only(&app);
    let code_host = code_only(&host);
    let code_modal = code_only(&modal);

    assert_eq!(
        code_host.matches("invoke_focus_undo_tree_trap").count(),
        1,
        "host.rs 里 invoke_focus_undo_tree_trap 必须恰好一处（apply_undo）"
    );
    assert_eq!(
        code_app
            .matches("undo-tree-focus-trapped: undo-tree-dialog.focus-trapped")
            .count(),
        1,
        "app.slint 里 undo-tree-focus-trapped 的绑定必须恰好一条"
    );
    assert_eq!(
        writes_of("undo-tree-focus-trapped", &app),
        0,
        "undo-tree-focus-trapped 必须是推导读数，全仓没有任何一处写它"
    );
    assert_eq!(
        code_modal.matches("FocusScope").count(),
        1,
        "UndoTreeModal 里必须恰好一个 FocusScope（陷阱本身）"
    );
    assert_eq!(
        code_modal.matches("accessible-id:").count(),
        8,
        "UndoTreeModal 的可寻址控件数必须一位不改（陷阱不设 accessible-id）"
    );
}

/// 只在**代码行**上数，注释里提到这个名字不算调用点（与 `tests/search_focus_key.rs` ⑤ 同口径）。
fn code_only(text: &str) -> String {
    text.lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 数「以单个 `=` 写这个标识符」的出现次数（`==` / `>=` / `<=` / `!=` 都不算写）。
fn writes_of(name: &str, source: &str) -> usize {
    let bytes = source.as_bytes();
    source
        .match_indices(name)
        .filter(|(offset, _)| {
            let offset = *offset;
            let after = offset + name.len();
            let independent = (offset == 0 || !is_ident(bytes[offset - 1]))
                && (after == bytes.len() || !is_ident(bytes[after]));
            let tail = source[after..].trim_start();
            independent && tail.starts_with('=') && !tail.starts_with("==")
        })
        .count()
}

fn is_ident(byte: u8) -> bool {
    (byte as char).is_alphanumeric() || byte == b'-' || byte == b'_'
}
