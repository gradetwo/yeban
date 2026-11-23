//! 契约面元判据（T-FIX-142）—— 扫描 `crates/yeban-app/tests/` 的源码。
//!
//! 放在 `yeban-model`（轻量 crate）而不是 `yeban-app/tests/` 的原因：`yeban-app` 的
//! test 目标要连 Slint 代码一起 codegen，本机 4 GB 内存下 rustc 会在 ~35 分钟处被 SIGKILL；
//! 本判据只读文本，放在轻 crate 里几十秒就能跑完，CI 上也不会成为新的长尾。：防止「用例名说行为、函数体只数源码字符串」的降级继续扩散。
//!
//! 背景：2026-11 的新 UI 重构里，`crates/yeban-app/tests/` 出现大批
//! `// 新 UI 重构: …` 标记 + `assert!(read_to_string(...).contains("..."))` 的用例。
//! 它们守的是「声明/接线还写在源码里」，而不是「行为真的成立」。三处真实缺陷
//! （弹窗打开不进焦点、Ctrl/Cmd+F 不聚焦、歌曲概览无数据）都在这类断言下长期全绿。
//!
//! 本文件两条规则（读源码，不需要跑 GUI）：
//! 1. 函数体里带 `新 UI 重构` 标记的用例，名字必须以 `contract_surface_` 开头 ——
//!    让「结构断言」与「行为断言」在命名上分得开，便于统计降级面。
//! 2. 用例名含行为动词（`moves_` / `opens_` / `focuses_` / `expands_` / `closes_` /
//!    `switches_` / `applies_` / `restores_` / `keeps_` / `routes_` / `scrolls_` /
//!    `pans_` / `drags_`），而函数体是「读源码 + contains」的纯文本断言，且没有
//!    `build_live_ui_with` / `live::` / `ui.ui()` 这类真实界面证据 ⇒ 判红。
//!    例外必须写进 `tests/contract_surface_whitelist.txt`（一行 `file.rs::fn_name  # 理由`），
//!    不许静默放过。
//!
//! ⚠ 本判据只做静态分类，不替代真实行为测试：它拦的是「名字骗人」，不是「行为正确」。

use std::path::{Path, PathBuf};

const PREFIX: &str = "contract_surface_";
const DEGRADE_MARK: &str = "新 UI 重构";
const BEHAVIOR_VERBS: &[&str] = &[
    "moves_",
    "opens_",
    "focuses_",
    "expands_",
    "closes_",
    "switches_",
    "applies_",
    "restores_",
    "keeps_",
    "routes_",
    "scrolls_",
    "pans_",
    "drags_",
];

fn tests_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../yeban-app/tests")
}

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// 极简 Rust 源码遍历：返回 `#[test]` / `#[tokio::test]` 标注的函数（名字 + 函数体原文）。
/// 只按花括号配对切分 —— tests 里的函数体不会出现字符串里的裸花括号干扰到这种程度的用例。
fn test_fns(src: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = src.as_bytes();
    for (start, _) in src.match_indices("fn ").filter(|(i, _)| {
        // 必须落在行首缩进后的 `fn`，避免把注释里的 “fn xxx” 也算进来
        let line_start = src[..*i].rfind('\n').map(|p| p + 1).unwrap_or(0);
        src[line_start..*i].trim().is_empty()
    }) {
        let name_start = start + 3;
        let name_end = src[name_start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map(|o| name_start + o)
            .unwrap_or(name_start);
        if name_end == name_start {
            continue;
        }
        let name = &src[name_start..name_end];
        let Some(brace) = src[name_end..].find('{').map(|o| name_end + o) else {
            continue;
        };
        let mut depth = 0usize;
        let mut end = None;
        for (off, b) in bytes.iter().enumerate().skip(brace) {
            match b {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(off);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else { continue };
        let body = &src[name_end..end];
        // 前 160 字节内必须有测试属性
        // 按**字符边界**回退：中文注释按字节切会 panic（本判据第一次跑就是死在这里）。
        let mut head_start = start.saturating_sub(160);
        while head_start > 0 && !src.is_char_boundary(head_start) {
            head_start -= 1;
        }
        let head = &src[head_start..start];
        if !(head.contains("#[test]") || head.contains("#[tokio::test]")) {
            continue;
        }
        out.push((name.to_string(), body.to_string()));
    }
    out
}

fn code_only(body: &str) -> String {
    body.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 「纯文本断言」：读源码 + `contains`，且没有任何真实界面/引擎调用的痕迹。
fn is_text_only(body: &str) -> bool {
    let code = code_only(body);
    if !code.contains(".contains(") {
        return false;
    }
    if !(code.contains("read_to_string") || code.contains("include_str!")) {
        return false;
    }
    const EVIDENCE: &[&str] = &[
        "build_live_ui_with",
        "live::",
        "LiveUi",
        "ui.ui()",
        "assert_eq!",
        "assert_ne!",
        "apply(",
        ".execute(",
        "set_",
        "get_",
    ];
    !EVIDENCE.iter().any(|e| code.contains(e))
}

fn whitelist() -> Vec<String> {
    let path = tests_dir().join("contract_surface_whitelist.txt");
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.split('#').next().unwrap_or("").trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
        Err(_) => Vec::new(),
    }
}

#[test]
fn degraded_assertions_are_named_contract_surface() {
    let mut violations = Vec::new();
    let mut files = Vec::new();
    rs_files(&tests_dir(), &mut files);
    for file in files {
        // 判据自身不算被测对象（它的正文里必须出现标记字面量）。
        if file
            .file_name()
            .is_some_and(|n| n == "contract_surface_meta.rs")
        {
            continue;
        }
        // `undo_tree_focus_trap.rs` 的空心化归 T-FIX-131：本单只登记、不改它。
        if file
            .file_name()
            .is_some_and(|n| n == "undo_tree_focus_trap.rs")
        {
            continue;
        }
        let src = std::fs::read_to_string(&file).expect("读 tests 源码");
        for (name, body) in test_fns(&src) {
            if code_only(&body).contains(DEGRADE_MARK) && !name.starts_with(PREFIX) {
                violations.push(format!(
                    "{}::{name}",
                    file.file_name().unwrap_or_default().to_string_lossy()
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "带 `新 UI 重构` 降级标记的用例必须以 `contract_surface_` 开头（否则降级面无法统计）：{violations:#?}"
    );
}

#[test]
fn behavior_named_tests_do_not_only_assert_source_text() {
    let allow = whitelist();
    let mut violations = Vec::new();
    let mut files = Vec::new();
    rs_files(&tests_dir(), &mut files);
    for file in files {
        // 判据自身不算被测对象（它的正文里必须出现标记字面量）。
        if file
            .file_name()
            .is_some_and(|n| n == "contract_surface_meta.rs")
        {
            continue;
        }
        // `undo_tree_focus_trap.rs` 的空心化归 T-FIX-131：本单只登记、不改它。
        if file
            .file_name()
            .is_some_and(|n| n == "undo_tree_focus_trap.rs")
        {
            continue;
        }
        let src = std::fs::read_to_string(&file).expect("读 tests 源码");
        for (name, body) in test_fns(&src) {
            if name.starts_with(PREFIX) {
                continue;
            }
            if !BEHAVIOR_VERBS.iter().any(|v| name.contains(v)) {
                continue;
            }
            let key = format!(
                "{}::{name}",
                file.file_name().unwrap_or_default().to_string_lossy()
            );
            if is_text_only(&body) && !allow.iter().any(|a| a == &key) {
                violations.push(key);
            }
        }
    }
    assert!(
        violations.is_empty(),
        "这些用例名说「行为」，函数体却只有源码文本 contains 断言：要么改 回真实界面断言（build_live_ui_with/live::），\
         要么改名 contract_surface_*，要么把理由写进 tests/contract_surface_whitelist.txt：{violations:#?}"
    );
}
