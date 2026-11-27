//! T-FIX-990：导出 durability 警告的 GUI 接线钉子（纯 `include_str!` 判据）。
//!
//! 纯函数（`midi_export_receipt_fields` / `midi_export_success_line` /
//! `stems_export_receipt_fields` / `toast_message`）的行为由 `src/host.rs` 与
//! `src/export_stems.rs` 的单元判据覆盖；本文件只钉**调用点**：两个导出臂必须
//! 真调这些纯函数（而不是各写一份内联字段表把警告丢掉 —— 那正是 990 修前的形态）。
//! 同款见 `tests/export_stems_gui.rs`（"纯 `include_str!` 判据"）。
//!
//! ## 判据怎么变红
//!
//! - MIDI 臂改回内联字段表（不调 `midi_export_receipt_fields`）⇒ ① 红；
//! - MIDI 臂改回旧提示条（不调 `midi_export_success_line`）⇒ ② 红；
//! - 分轨臂改回内联字段表（不调 `stems_export_receipt_fields`）⇒ ③ 红；
//! - `humanize_module_receipt` 删掉持久化警告子句 ⇒ ④ 红。
//!
//! 钉子必须带调用点上下文（`module_receipt("export-midi", …)` / `let success_line = …` /
//! `format!("（持久化警告…`）—— 光钉函数名会被 `host.rs` 自身单元测试里的
//! `super::…(&report)` 调用顶住，永远变不红（T-FIX-990 红对照实测）。

const HOST: &str = include_str!("../src/host.rs");

#[test]
fn midi_export_arm_goes_through_the_durability_aware_receipt_builder() {
    assert!(
        HOST.contains("module_receipt(\"export-midi\", &midi_export_receipt_fields(&report))"),
        "on_export_midi 的成功臂必须调 midi_export_receipt_fields（内联字段表会把 durability_warning 丢掉）"
    );
}

#[test]
fn midi_export_arm_goes_through_the_durability_aware_success_line() {
    assert!(
        HOST.contains("let success_line = midi_export_success_line(&report);"),
        "on_export_midi 的成功臂必须调 midi_export_success_line（旧提示条不带持久化警告）"
    );
}

#[test]
fn stems_export_arm_goes_through_the_durability_aware_receipt_builder() {
    assert!(
        HOST.contains("stems_export_receipt_fields(&report, &out_for_msg)"),
        "export-stems 的成功臂必须调 stems_export_receipt_fields（内联字段表只有计数、没有警告原文）"
    );
}

#[test]
fn humanize_module_receipt_carries_the_durability_clause() {
    assert!(
        HOST.contains("format!(\"（持久化警告：{warning}）\")"),
        "humanize_module_receipt 必须把 durability_warning 字段缀进人话行（否则状态栏常驻行看不到警告）"
    );
}
