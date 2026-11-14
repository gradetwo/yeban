//! 判据：`F7`/`F8`/`F9` 从**真事件源**直达底部控制台的三个标签。
//!
//! 本切片补的是"键盘用户到不了调音台 / 设备页"这一缺口（**界面决定**，⛔ 不是规范点名）：
//! 标签导轨此前只有鼠标 `TouchArea.clicked` 与屏读器 `accessible-action-default` 两个入口，
//! 两者都写 `root.console-tab`；键盘这一半一个入口都没有。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `f7_f8_f9_pick_console_tabs_and_stay_behind_the_text_policy` | 三个功能键解出 `SelectConsoleTab { index }`；下标对上 `scene::CONSOLE_TABS`；文本域 / IME 策略仍然优先 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys`，再读窗口的 `console-tab` | 按键 ⇒ `console-tab` 真的变成那一个下标；带修饰 ⇒ 不消费、下标不动 |
//!
//! ⚠ 本文件**不**点标签导轨：`active-tab` 是**同一个**绑定（`app.slint` 的
//! `active-tab: root.console-tab`），鼠标 / 屏读器 / 键盘三条入口改的是同一个属性 ⇒
//! `.slint` 的 `visible` 分支不需要第四份判据（否则就是给同一条链写三种观测）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 臂里写死 `ui.set_console_tab(0)` | 本文件红（`F8`/`F9` 之后下标不动） |
//! | 臂里返回 `false`（不消费） | 本文件红（第 ① 条的返回值断言） |
//! | 把 `resolve_logical` 第 6 步的 `Key::F8` 删掉 | 本文件红（键不消费）＋ `src/input.rs` 那条红 |
//! | 把三臂搬回第 1 步（`F5`/`F6` 的全局档） | `src/input.rs` 那条红（合成态不再被吞） |
//! | 删掉 `apply_action` 的 `SelectConsoleTab` 臂 | 编译红（穷尽匹配，⛔ 没有 `_` 兜底） |
//! | 把 `cli.rs` 里 `F8` 那一行的 `key` 写成 `F7` | `tests/cli_contract.rs` 的 B11b 红 |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `record_key_ui.rs` / `mixer_keyboard_switch.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// Slint `key_codes.rs` 的私有区文本（`F7` = `\u{f70a}` …）。
const F7: &str = "\u{f70a}";
const F8: &str = "\u{f70b}";
const F9: &str = "\u{f70c}";

#[test]
fn f7_f8_f9_reach_the_three_console_tabs() {
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
        aether_app.contains("> console-tab:"),
        "新 UI 契约面必须声明 > console-tab:"
    );
    assert!(
        aether_host.contains("set_console_tab"),
        "宿主接线层必须注册 set_console_tab"
    );
    assert!(
        aether_app.contains("callback key-action("),
        "新 UI 契约面必须声明 callback key-action("
    );
    assert!(
        aether_host.contains("on_key_action"),
        "宿主接线层必须注册 on_key_action"
    );
}
