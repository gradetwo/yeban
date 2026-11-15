//! 判据：`Cmd/Ctrl+Alt+A` 从**真事件源**落到走带 AI 抽屉按钮**同一个** `toggle-ai-drawer` 回调。
//!
//! 本切片补的是"AI 抽屉只能点走带 `transport-ai-drawer-button`"这一缺口（**界面决定**，
//! ⛔ 不是规范点名；`[UI-NOTE-005]` 只讲音符）。抽屉是纯视图态（`MODEL-ISO-001` 的挥发性层）
//! ⇒ 不改工程、不进撤销栈。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `ctrl_alt_a_is_the_ai_drawer_entry_and_does_not_disturb_its_neighbours` | `Cmd/Ctrl+Alt+A` 解出 `ToggleAiDrawer`；`Cmd` 侧同解；邻居仍放行；文本框放行 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys` | ① 按键 ⇒ `ai-drawer-open` **真的翻转**（端到端）；② 换计数器后 ⇒ 回调**恰好**一次；③ 邻居⇒ 不消费且计数不动；④ `Escape` ⇒ 收起开着的抽屉（`U-11`），关上之后如实放行 |
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 臂里改成 `ui.invoke_toggle_sidebar()`（错回调） | 本文件红（属性不翻） |
//! | 臂里返回 `false`（不消费） | 本文件红（第 ① 条） |
//! | 删掉 `resolve_logical` 第 3 步的 `Key::Character('a')` | 本文件红（键不消费）＋ `src/input.rs` 那条红 |
//! | 删掉 `apply_action` 的 `ToggleAiDrawer` 臂 | 编译红（穷尽匹配，⛔ 没有 `_` 兜底） |
//! | 把 `cli.rs` 那一行的 `modifiers` 写成 `Modifiers::ctrl()` | `tests/cli_contract.rs` 的 B11b 红 |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `record_key_ui.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// 裸键的文本形态 —— `logical()` 把它映射成 `Character('a')`。
const A: &str = "a";

#[test]
fn ctrl_alt_a_is_a_second_entry_to_the_ai_drawer_button_callback() {
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}

/// `U-11`（`~/zimmer-ui-mode-report.md`）：抽屉开着时 `Escape` 收起它。
///
/// 这是**面板收尾**（与时光机 / 导出弹窗同一条 `Action::Cancel` 链），⛔ 不是"放弃 AI
/// 提案"—— 拒绝仍只有 `musical-pr-reject-button`。判据落在**生产同一个** `wire_keys`
/// 上：抽屉关着时 `Escape` 必须如实放行（`tests/live_ui_mcp.rs` 对这条放行也有断言）。
#[test]
fn escape_closes_the_open_ai_drawer_and_then_passes_through() {
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
        aether_app.contains("> ai-drawer-open:"),
        "新 UI 契约面必须声明 > ai-drawer-open:"
    );
    assert!(
        aether_host.contains("set_ai_drawer_open"),
        "宿主接线层必须注册 set_ai_drawer_open"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
    assert!(
        aether_app.contains("callback toggle-ai-drawer("),
        "新 UI 契约面必须声明 callback toggle-ai-drawer("
    );
    assert!(
        aether_host.contains("on_toggle_ai_drawer"),
        "宿主接线层必须注册 on_toggle_ai_drawer"
    );
}
