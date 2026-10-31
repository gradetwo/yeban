//! **常驻判据（R93／R100）**：`src/**` 里用 `` `路径::name` `` **指名**的判据必须真的存在。
//!
//! # 为什么有它（2026-10-30 实测）
//!
//! `domain/render.rs` 的模块文档是"已知不支持项"的**唯一事实源**，它两处写着
//! `render::tests::every_known_unsupported_key_is_actually_reported`。
//! 而 `grep -rn 'fn every_known' crates/` ＝ **0 行** ⇒ 这个名字是**死的**；
//! 真判据叫 `the_known_unsupported_list_is_not_decoration`（`tests/render_master.rs`）。
//! 后果不是"少一条判据"，而是**那句承诺无可核**：照文档去核对的人什么也找不到，
//! 于是"清单不会腐烂成装饰表"这句话在事实源里**无法被跟随**。
//!
//! `documented_arg_coverage.rs` 已经在**登记表**上拦这一类（`defines_fn`），但它只查
//! 表里登记的名字；**散文里的引用**此前没有任何判据在看 ⇒ 本文件补的就是这一格。
//!
//! # 口径（机械、可复核 ⛔ 不是黑箱）
//!
//! - **引用** ＝ `::tests::NAME` 或 `.rs::NAME` 形状（`NAME` 取到第一个非标识符字符为止）；
//!   只从 `src/**` 里取 —— 那是**对外承诺**的所在；`tests/**` 里提到别的判据是叙述。
//! - **存在** ＝ `crates/yeban-mcp` 下**任一个** `.rs` 里有 `fn NAME(`（`src` 与 `tests` 都算，
//!   因为判据既有内联模块测试也有集成测试）。
//! - **非真空**：引用数必须 ≥ 当前读数 **29**（R93 棘轮：删引用要同步下调此界并在提交
//!   说明写明理由），扫到的源文件数必须 ≥ 下界 40（R100：扫描面不能悄悄缩水）。
//!   ⚠ 想缩面的**真实**保护是 29 这个引用数下界 —— 删掉一个源文件，它里面的引用随之消失，
//!   引用数就掉下来；文件数下界只负责"扫描器没失灵"。
//!
//! # 注入自证（改坏就会红）
//!
//! 把任意一处引用的名字改成 `zzGhostCriterion` ⇒ 本判据点名 `文件 ⇒ 名字`。

use std::fs;
use std::path::{Path, PathBuf};

/// 本 crate 的根（`CARGO_MANIFEST_DIR` ＝ `crates/yeban-mcp`）。
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 递归收集 `.rs`（⛔ 不读 `target/`：只走我们传入的两个根）。
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

/// 抽出 `text` 里被**指名**的判据名（`::tests::NAME` ／ `.rs::NAME`）。
fn referenced_criteria(text: &str) -> Vec<String> {
    const MARKS: [&str; 2] = ["::tests::", ".rs::"];
    let mut out = Vec::new();
    for mark in MARKS {
        let mut rest = text;
        while let Some(index) = rest.find(mark) {
            let after = &rest[index + mark.len()..];
            let name: String = after
                .chars()
                .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                .collect();
            if !name.is_empty() {
                out.push(name);
            }
            rest = after;
        }
    }
    out
}

/// 恒定输出：引用名逐条打印（⛔ 让抽取器变成黑箱的读数都没有）。
fn assert_all_resolve(references: &[(String, String)], texts: &[(PathBuf, String)]) {
    let unresolved: Vec<String> = references
        .iter()
        .filter(|(_, name)| {
            let needle = format!("fn {name}(");
            !texts.iter().any(|(_, text)| text.contains(&needle))
        })
        .map(|(path, name)| format!("{path} ⇒ `{name}`"))
        .collect();
    assert!(
        unresolved.is_empty(),
        "这些散文中点名的判据在整 crate 里**不存在**（找不到 `fn NAME(`）:\n{}\n\
         ⇒ 要么把名字改成真判据（并核对它确实在测那件事），要么把判据补上；\
         ⛔ 不许留一张死名册（`documented_arg_coverage` 在**登记表**上拦的是同一类）",
        unresolved.join("\n")
    );
    println!(
        "### 散文判据引用：{} 处（逐条 {:?}）、扫过 {} 个 `.rs`、未解析 0 处",
        references.len(),
        references
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>(),
        texts.len()
    );
}

#[test]
fn every_criterion_named_in_the_sources_exists() {
    let root = crate_root();
    let src_dir = root.join("src");
    let mut files = Vec::new();
    rs_files(&src_dir, &mut files);
    rs_files(&root.join("tests"), &mut files);
    assert!(
        files.len() >= 40,
        "只扫到 {} 个 `.rs`（src ＋ tests）⇒ 扫描面异常（现读远多于 40）",
        files.len()
    );

    let mut texts: Vec<(PathBuf, String)> = Vec::new();
    for path in &files {
        match fs::read_to_string(path) {
            Ok(text) => texts.push((path.clone(), text)),
            Err(error) => panic!("{} 读不出来: {error}", path.display()),
        }
    }

    let mut references: Vec<(String, String)> = Vec::new();
    for (path, text) in &texts {
        if !path.starts_with(&src_dir) {
            continue;
        }
        for name in referenced_criteria(text) {
            references.push((path.display().to_string(), name));
        }
    }
    // ① R93／R100：引用数必须达到下界（⛔ 抽取规则失灵 / 指路被整体删掉却静默通过）。
    assert!(
        references.len() >= 29,
        "`src/**` 里的判据引用只剩 {} 处（现读 29 处）⇒ 抽取规则失灵，或指路被整体删掉 \
         （⛔ 静默通过：删引用要同步下调此界并写明理由）",
        references.len()
    );

    // ② 名字必须能落到一个真函数上。
    assert_all_resolve(&references, &texts);
}

/// 抽取器本体：**运行期构造**针与期望形态（R84：⛔ 把期望字面量写进 `contains` 实参）。
#[test]
fn the_extractor_recognises_both_reference_shapes() {
    let module_tests = format!("{}{}{}", "x::", "tests::", "alpha_case");
    let file_tests = format!("{}{}{}", "render_master", ".rs::", "beta_case");
    let prose = format!("见 `{module_tests}` 与 `{file_tests}` 两条");
    let got = referenced_criteria(&prose);
    let expected = vec!["alpha_case".to_string(), "beta_case".to_string()];
    assert_eq!(
        got, expected,
        "抽取器必须认出两种形状（模块测试 / 集成测试文件）"
    );
}
