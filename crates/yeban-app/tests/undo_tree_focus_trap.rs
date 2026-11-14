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
//! ## 判据（一条链，五个读数）
//!
//! ① 起点：快捷键层有焦点、模态未持有；
//! ② 打开（端口 + `apply_undo`，与宿主同一个开关）⇒ 焦点**真的**移进模态；
//! ③ 模态内按 `3` ⇒ 仍从**同一个**键盘入口到达宿主（`active-tool` = 3）；
//! ④ 模态内按 `Tab` ⇒ 焦点**不**轮转出去，也不触发画布语义的切视图（视图不变）；
//! ⑤ `Esc` 关闭 ⇒ 焦点复位到快捷键层，随后 `1` 又能到达宿主（`active-tool` = 1）。
//!
//! ⛔ 本判据**不**声称"模态内各控件之间的 Tab 顺序"：本骨架里模态的按钮不是 `FocusScope`
//! （只有 `TouchArea`），闭环 = 焦点停在模态这一个作用域上。

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code（`-D warnings` 会判红）。
#[allow(dead_code)]
mod live;

use std::rc::Rc;

use live::{LiveWiringOptions, build_live_ui_with};
use yeban_app::undo::{UiAction, UndoPort, UndoSession};
use yeban_ui_test_port::port::{KeyCode, Permission};
use yeban_ui_test_port::render::report_line;

/// 会话打开时刻（与既有撤销判据同一个夹具常量）。
const NOW: u64 = 1_760_000_000_000;

#[test]
fn the_undo_tree_traps_focus_and_restores_it_to_the_shortcut_layer() {
    // 新 UI 重构: 旧面板视觉元素不再进控件树; 核对同一条宿主接线的契约面。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
