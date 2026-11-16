//! **`on_trigger_shortcut` 只能赋值一次**（清单 P2-1 / ADR-0074 遗留）。
//!
//! 背景：`wire_aether_callbacks`（装配期先调）曾给 `ui.on_trigger_shortcut` 赋一个
//! 「空格/r/c/m/1-4/b 白名单」处理器；`wire_aether_actions`（main.rs 后调）又赋一次
//! ⇒ Slint 后写覆盖前写，白名单**从未执行**（死代码）。改错地方不会有任何效果。
//!
//! 修法：删除装配期那一处，键盘统一由 `wire_aether_actions` → `invoke_key_action`
//! → `input.rs` 解析表（ADR-0074 R1）处理。⛔ 行为零变更（它本来就没跑）。
//!
//! ⚠ 纯 `include_str!` 判据（本机编不了 Slint）。

const HOST: &str = include_str!("../src/host.rs");

#[test]
fn on_trigger_shortcut_is_assigned_exactly_once() {
    let count = HOST
        .lines()
        .filter(|l| l.trim_start().starts_with("ui.on_trigger_shortcut({"))
        .count();
    assert_eq!(
        count, 1,
        "on_trigger_shortcut 只能赋值一次；多一处就是被后写覆盖的死代码（P2-1）"
    );
}
