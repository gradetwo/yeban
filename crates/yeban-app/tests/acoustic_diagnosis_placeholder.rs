//! **占位 UI 诚实性判据（T-FIX-425，路线 b）** —— 「执行 AI 声学检查与诊断」
//! 没有任何声学分析实现，却曾在菜单里可点、并在无头装配里弹一条假完成 toast。
//!
//! 处置（路线 b，不做 a）：在真实声学分析落地前，两个入口禁用、假 toast 的注册
//! 删除、回调仍只打 stderr。本文件把这三点钉成源码判据；谁真接了声学分析，
//! 就必须把两处禁用解除并同步更新本文件（见 `docs/ledger/feature-alignment.md`
//! 错位 7 的 2026-11-25 加注与 `main.rs` 的 `wire_callbacks` 文档注释）。
//!
//! | 判据 | 钉住什么 |
//! | :--- | :--- |
//! | [`menu_item_run_acoustic_diagnosis_is_disabled`] | 诊断菜单项带 `enabled: false`，点不下去 |
//! | [`ai_rail_diagnose_button_is_disabled_and_rail_hidden`] | AI 轨道诊断键的 `TouchArea` 带 `enabled: false`，且整条 AI 轨道仍藏在 `if false` 里 |
//! | [`no_fake_acoustic_completion_toast`] | `声学诊断完成` 这串假结论字面量不得回到 `src/` 与 `ui/` 的任何代码文本里 |
//! | [`run_acoustic_diagnosis_has_exactly_one_trace_only_registration`] | `on_run_acoustic_diagnosis` 在 `src/` 里恰好一处注册，且是 `main.rs` 的 trace-only 那一行 |
//!
//! 判据住在 `tests/` 而不是 `src/elements.rs` 的单元测试里：它们读的是源码文本，
//! 与 `undo_tree_honesty.rs` 同款（本机不渲染 Slint 时也能跑）。
//!
//! 注入证明（红对照）：把菜单项的 `enabled: false` 删掉 ⇒ 判据 1 红；
//! 把 `TouchArea` 的 `enabled: false` 删掉 ⇒ 判据 2 红；把假 toast 注册写回
//! `host::wire_aether_callbacks` ⇒ 判据 3 与 4 同时红。

// assertion_bounds_audit 禁止新增 .all( 站点 ⇒ 此处用循环，
// clippy::unnecessary_fold 的常规建议与本仓守卫冲突，具名豁免。
#![allow(unused, clippy::unnecessary_fold)]
use std::path::{Path, PathBuf};

/// 假完成结论的字面量（逐字）。修前 `host::wire_aether_callbacks` 里
/// `on_run_acoustic_diagnosis` 的 toast 文案；仓库里没有任何声学分析，
/// 这句话是凭空捏造的结论，不得回到任何代码文本里。
const FAKE_COMPLETION: &str = "声学诊断完成";

/// `crates/yeban-app/`。
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read(rel: &str) -> String {
    let path = crate_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()))
}

/// 去掉 `//` 行注释与行尾注释后的**代码文本**。
///
/// 为什么要去注释：判据的留痕注释里会逐字提到被删掉的东西（审计留痕）。
/// 探针若把注释也算成「代码里的字面量」，就会逼着作者**不写**留痕 ——
/// 那与本仓库的纪律相反（与 `undo_tree_honesty.rs` 的 `code_only` 同款）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _comment)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `dir` 下指定后缀的全部文件（相对 `crates/yeban-app/` 的路径 + 源码）。
fn all_files_with_ext(dir: &str, ext: &str) -> Vec<(String, String)> {
    fn walk(dir: &Path, ext: &str, root: &Path, out: &mut Vec<(String, String)>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("读目录 {} 失败: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                walk(&path, ext, root, out);
            } else if path.extension().is_some_and(|e| e == ext) {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()));
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                out.push((rel, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&crate_dir().join(dir), ext, &crate_dir(), &mut out);
    out
}

#[test]
fn menu_item_run_acoustic_diagnosis_is_disabled() {
    let app = read("ui/app.slint");
    let mut found = 0;
    for line in app.lines() {
        if line.contains("MenuItem") && line.contains("run-acoustic-diagnosis") {
            found += 1;
            assert!(
                line.contains("enabled: false"),
                "诊断菜单项必须禁用（`enabled: false`）：{line}"
            );
        }
    }
    assert!(found >= 1, "ui/app.slint 里找不到声学诊断菜单项");
}

#[test]
fn ai_rail_diagnose_button_is_disabled_and_rail_hidden() {
    let app = read("ui/app.slint");
    let lines: Vec<&str> = app.lines().collect();

    // ① 整条 AI 轨道仍藏在 `if false` 里：`ai-rail` 区域往上数几行之内
    // 必须是 `if false : Rectangle {` 这一行开的块。
    let mut rail_idx = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.contains("accessible-id: \"ai-rail\";") {
            rail_idx = Some(idx);
            break;
        }
    }
    let rail_idx = rail_idx.expect("ui/app.slint 里找不到 ai-rail 区域");
    let mut hidden = false;
    let mut idx = rail_idx;
    while idx > 0 && rail_idx - idx <= 5 {
        idx -= 1;
        if lines[idx].trim_start().starts_with("if false") {
            hidden = true;
            break;
        }
    }
    assert!(hidden, "ai-rail 区域必须仍藏在 `if false` 块里");

    // ② 诊断键的 TouchArea 必须禁用：万一哪天轨道被重新显示出来，
    // 这个键也不能再点下去。
    let mut button_idx = None;
    for (idx, line) in lines.iter().enumerate() {
        if line.contains("accessible-id: \"ai-rail-diagnose-button\";") {
            button_idx = Some(idx);
            break;
        }
    }
    let button_idx = button_idx.expect("ui/app.slint 里找不到 ai-rail-diagnose-button");
    let mut touch = None;
    for line in lines.iter().skip(button_idx).take(10) {
        if line.contains("TouchArea") {
            touch = Some(*line);
            break;
        }
    }
    let touch = touch.expect("ai-rail-diagnose-button 块里找不到 TouchArea");
    assert!(
        touch.contains("enabled: false"),
        "ai-rail-diagnose-button 的 TouchArea 必须禁用（`enabled: false`）：{touch}"
    );
}

#[test]
fn no_fake_acoustic_completion_toast() {
    let mut hits = Vec::new();
    for (rel, text) in all_files_with_ext("src", "rs") {
        if code_only(&text).contains(FAKE_COMPLETION) {
            hits.push(rel);
        }
    }
    for (rel, text) in all_files_with_ext("ui", "slint") {
        if code_only(&text).contains(FAKE_COMPLETION) {
            hits.push(rel);
        }
    }
    assert!(
        hits.is_empty(),
        "假完成结论 `{FAKE_COMPLETION}` 不得回到代码文本里：{}",
        hits.join(", ")
    );
}

#[test]
fn run_acoustic_diagnosis_has_exactly_one_trace_only_registration() {
    let mut registrations: Vec<String> = Vec::new();
    for (rel, text) in all_files_with_ext("src", "rs") {
        for line in code_only(&text).lines() {
            if line.contains("on_run_acoustic_diagnosis") {
                registrations.push(format!("{rel}: {}", line.trim()));
            }
        }
    }
    assert_eq!(
        registrations.len(),
        1,
        "`on_run_acoustic_diagnosis` 在 src/ 里必须恰好一处注册，实际 {} 处：{}",
        registrations.len(),
        registrations.join(" | ")
    );
    assert!(
        registrations[0].contains("trace("),
        "唯一的一处注册必须是 trace-only（main.rs 的未接线打点）：{}",
        registrations[0]
    );
}
