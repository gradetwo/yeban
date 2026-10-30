//! **常驻判据（R93／R100）**：`crates/yeban-engine/src/**` 的文档里点名的判据必须**真的存在**。
//!
//! # 为什么要有它
//!
//! `src/mixer.rs:150` 曾写"两者的对应关系由 [`PanLaw::from_model`] 与
//! `tests/mix_render.rs::engine_pan_law_names_match_the_model`（穷举四个变体）钉住"——
//! 而 `fn engine_pan_law_names_match_the_model(` 在**整 crate** 里搜不到，`mix_render.rs`
//! 当时对 `PanLaw` **零提及**。照名字去核对的人**什么都找不到**，那句"穷举钉住"没有载体。
//! 同一类缺陷在 `crates/yeban-mcp` 已被 `tests/doc_criterion_references_resolve.rs` 常驻钉住
//! （台账 §14），本文件把这条纪律扩到 engine。
//!
//! # 口径
//!
//! * 只扫 **`src/**`**：`tests/**` 里出现的名字是叙述，不是承诺；
//! * 认两种指路形状：`::tests::NAME` 与 `.rs::NAME`；
//! * `NAME` 必须能落到 **`fn NAME(`** 上。解析语料 ＝ 本 crate（`src` ＋ `tests`）
//!   ∪ `yeban-dsp`：后者是唯一被 engine 散文**显式跨 crate** 指路的目标
//!   （`src/level.rs:30` 的 `yeban_dsp::meter::tests::…`，电平实现已上移到 dsp）。
//!   语料再放宽会把"写错 crate"的情况也放绿，所以⛔ 不要扩到整个 workspace；
//! * 反向非真空：被扫文件数、引用总数、去重名字数各有下界（＝当前值），
//!   删文件／删引用必须同步下调并写明理由。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 指路形状（与 mcp 侧守卫同口径）。
const MARKS: [&str; 2] = ["::tests::", ".rs::"];

fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn referenced_criteria(text: &str) -> Vec<String> {
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

#[test]
fn every_criterion_named_in_the_engine_sources_exists() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));

    let mut files = Vec::new();
    rs_files(&manifest.join("src"), &mut files);
    // R100：下界 ＝ 当前 `src/**` 的 `.rs` 文件数。删文件会让本判据变松 ⇒ 必须同步下调。
    assert!(
        files.len() >= 24,
        "被扫的 `src/**` 文件数 {} 必须 ≥ 24（＝当前值；新增文件时抬高，删文件必须写明理由）",
        files.len()
    );

    let mut corpus = Vec::new();
    rs_files(manifest, &mut corpus);
    rs_files(&manifest.join("../yeban-dsp"), &mut corpus);
    assert!(
        corpus.len() >= 116,
        "解析语料（engine src+tests ∪ yeban-dsp）{} 个 `.rs` 必须 ≥ 116（＝当前值）",
        corpus.len()
    );
    let mut texts = Vec::new();
    for path in &corpus {
        if let Ok(text) = std::fs::read_to_string(path) {
            texts.push(text);
        }
    }

    let mut references = Vec::new();
    for file in &files {
        let source = std::fs::read_to_string(file)
            .unwrap_or_else(|error| panic!("读不到 {}: {error}", file.display()));
        for name in referenced_criteria(&source) {
            references.push((file.clone(), name));
        }
    }

    // 反向非真空：引用与去重名字都要有下界（现读 7 处 / 4 个名字）。
    assert!(
        references.len() >= 7,
        "散文指路引用数 {} 必须 ≥ 7（删引用要同步下调并写明理由）",
        references.len()
    );
    let distinct: BTreeSet<&str> = references.iter().map(|(_, name)| name.as_str()).collect();
    assert!(
        distinct.len() >= 4,
        "去重后的被指名字 {} 个必须 ≥ 4",
        distinct.len()
    );

    let mut unresolved = Vec::new();
    for (file, name) in &references {
        let needle = format!("fn {name}(");
        if !texts.iter().any(|text| text.contains(&needle)) {
            unresolved.push(format!("{}: {name}", file.display()));
        }
    }
    assert!(
        unresolved.is_empty(),
        "以下散文指路的判据在解析语料里找不到 `fn 名字(` ⇒ 照名字去核对的人会一无所获：\n{}",
        unresolved.join("\n")
    );
}

/// 抽取器自证（R84：⛔ 把期望字面量写进 `contains` 实参）：两种形状的针在**运行期**拼出来。
#[test]
fn the_extractor_recognises_both_reference_shapes() {
    let mark_a = concat!("::", "tests::");
    let mark_b = concat!(".", "rs::");
    let name_a = concat!("name_", "from_tests");
    let name_b = concat!("name_", "from_file");
    let text = format!("见 {mark_a}{name_a} 与 {mark_b}{name_b}");

    let found = referenced_criteria(&text);
    assert_eq!(found.len(), 2, "两种形状各应抽出一个名字，实得 {found:?}");
    assert!(found.contains(&name_a.to_owned()));
    assert!(found.contains(&name_b.to_owned()));
}
