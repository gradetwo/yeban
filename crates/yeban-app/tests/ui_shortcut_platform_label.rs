//! `--print-shortcuts` 键位表「当前平台单写」的源码层看守
//! （控件体检 §2.5 / §8 P1、细节实测 §5.11）。
//!
//! 两份报告都点名：策略表、菜单文案、tooltip 一律 `Cmd/Ctrl+X` 混排，
//! Windows 用户看到 `Cmd` 会先愣一下，两套修饰键写在一个字符串里可读性也差。
//! 菜单/tooltip 已在别处收口成 `Ctrl+`，本票收口 `--print-shortcuts` 主表。
//!
//! 修法刻意在**渲染层**做平台化，而不是把数据表里的 `Cmd/Ctrl+` 机械删掉：
//! `SHORTCUTS` 表保留跨平台语义（判据与文档逐字依赖它），只有面向人阅读的
//! 输出按 `cfg!(target_os = "macos")` 折叠成 `Cmd+`（mac）/ `Ctrl+`（其余）。
//!
//! 本文件只读 `cli.rs` 源码文本（⛔ 不编译 Slint）：任一处把渲染层改回裸
//! `row.label.to_owned()`、或把数据表机械替换掉，对应判据立即变红。

#![allow(unused)]

const CLI: &str = include_str!("../src/cli.rs");

/// 取 `shortcut_lines` 函数体（到下一个 `///` 文档块之前）。
fn shortcut_lines_body() -> &'static str {
    let start = CLI
        .find("pub fn shortcut_lines()")
        .expect("cli.rs 必须有 shortcut_lines");
    let tail = &CLI[start..];
    // 截到函数结束的 `\n}\n`
    let end = tail.find("\n}\n").expect("shortcut_lines 必须有结尾");
    &tail[..end]
}

/// **判据 ①**：存在按平台折叠的 helper，且两支分别给 Cmd+ / Ctrl+。
#[test]
fn platform_single_modifier_helper_exists() {
    assert!(
        CLI.contains("fn shortcut_label_for_platform(label: &str) -> String"),
        "必须有把规范写法按平台单写的 helper"
    );
    assert!(
        CLI.contains("cfg!(target_os = \"macos\")"),
        "helper 必须按 macOS / 其余平台分支"
    );
    assert!(CLI.contains("\"Cmd+\""), "macOS 分支应单写 Cmd+");
    assert!(CLI.contains("\"Ctrl+\""), "其余平台分支应单写 Ctrl+");
    assert!(
        CLI.contains("label.replace(\"Cmd/Ctrl+\", single)"),
        "helper 必须把 Cmd/Ctrl+ 折叠成 single"
    );
}

/// **判据 ②**：`shortcut_lines` 渲染层用平台化后的 label，⛔ 不再裸用 `row.label`。
#[test]
fn shortcut_lines_renders_the_platform_label() {
    let body = shortcut_lines_body();
    assert!(
        body.contains("let rendered = shortcut_label_for_platform(row.label);"),
        "渲染层必须调用平台化 helper"
    );
    assert!(
        !body.contains("row.label.to_owned()"),
        "渲染层不得再直接输出裸 row.label（否则 Cmd/Ctrl+ 混排又回来）"
    );
    assert!(
        body.contains("let label = if row.implemented {\n            rendered"),
        "implemented 分支必须用 rendered"
    );
}

/// **判据 ③**：数据表 `SHORTCUTS` 仍保留 `Cmd/Ctrl+` 跨平台语义（未被机械删除）。
#[test]
fn shortcut_data_table_keeps_cross_platform_labels() {
    let label_rows = CLI
        .lines()
        .filter(|l| l.contains("label: \"Cmd/Ctrl+"))
        .count();
    assert!(
        label_rows >= 20,
        "数据表应仍含多条 Cmd/Ctrl+ label（跨平台语义保留），实测 {label_rows} 条"
    );
}
