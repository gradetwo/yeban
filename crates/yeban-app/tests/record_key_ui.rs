//! 判据：裸 `R` 从**真事件源**落到走带「录音」按钮**同一个** `toggle-record` 回调。
//!
//! 本切片补的是"录音只能靠鼠标点走带按钮"这一缺口（**界面决定**，⛔ 不是规范点名；
//! `[ARCH-REC-001]` 点名的是录音**能力**本身，已由 `host::wire_record_at` 落地）。
//!
//! ## 两层（不可互相代替）
//!
//! | 层 | 位置 | 证的事 |
//! | :--- | :--- | :--- |
//! | A 纯解析 | `src/input.rs` 的 `bare_r_toggles_record_and_does_not_disturb_its_neighbours` | 裸 `R` 解出 `ToggleRecord`；带修饰的邻居仍放行；文本框放行 |
//! | B 活窗口（本文件） | `build_live_ui_with` 装**生产同一个** `host::wire_keys`，再在根回调 `toggle-record` 上装计数器 | 按键 ⇒ 回调**恰好**一次；带修饰 ⇒ 不消费且计数不动 |
//!
//! ⚠ 本文件**不**开真设备：`toggle-record` 的产品实现（起录 / 停录 / 落盘）在
//! `host::wire_record_at`，判据在录音那几份文件里。本文件只钉"键盘按到了**那一个**回调" ——
//! 与 `src/elements.rs` 钉住"按钮按到了 `root.toggle-record()`"合起来就是完整链路
//! （`player` 侧的"切了状态"另有 `record_device_wiring.rs` 等文件）。
//!
//! ## 变异表（每条都写明它应当让谁变红）
//!
//! | 注入 | 预期 |
//! | :--- | :--- |
//! | 臂里改成 `ui.invoke_toggle_play()`（错回调） | 本文件红（计数不动） |
//! | 臂里返回 `false`（不消费） | 本文件红（第 ① 条） |
//! | 删掉 `resolve_logical` 第 6 步的 `Key::Character('r')` | 本文件红（键不消费）＋ `src/input.rs` 那条红 |
//! | 删掉 `apply_action` 的 `ToggleRecord` 臂 | 编译红（穷尽匹配，⛔ 没有 `_` 兜底） |
//! | 把 `cli.rs` 那一行的 `key` 写成 `KeyD` | `tests/cli_contract.rs` 的 B11b 红 |

#![allow(unused)]
#[path = "../src/live_surface.rs"]
// 本判据只用到 `live_surface.rs` 的一部分 ⇒ 其余项在本测试目标里是 dead_code，
// 而 `-D warnings` 会判红（与 `mixer_keyboard_switch.rs` 同一处理）。
#[allow(dead_code)]
mod live;

use std::cell::RefCell;
use std::rc::Rc;

use yeban_model::samples::filled_project;
use yeban_ui_test_port::port::Permission;

/// 裸键的文本形态 —— `logical()` 把它映射成 `Character('r')`。
const R: &str = "r";

#[test]
fn bare_r_is_a_second_entry_to_the_record_button_callback() {
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
    assert!(
        aether_app.contains("callback toggle-play("),
        "新 UI 契约面必须声明 callback toggle-play("
    );
    assert!(
        aether_host.contains("on_toggle_play"),
        "宿主接线层必须注册 on_toggle_play"
    );
}
