//! **`Op` 变体的第三口径**：工具面可达性（源码级普查）+ `ops[].kind` 的双射守卫。
//!
//! ## 三个口径的区别（本文件只做第三口径）
//!
//! | 口径 | 问题 | 弱点 |
//! | :--- | :--- | :--- |
//! | ① 构造点 | `src` 里有没有 `Op::V {`？ | 含 **match 模式**，因此是宽松下限 |
//! | ② 任何提及 | `src` 里有没有提到 `Op::V`？ | 含**文档注释** |
//! | ③ 工具面可达 | **哪个工具的产线真的构造它**？ | 必须先分辨"构造"与"模式" |
//!
//! 本文件把 ③ 变成机制：对**工具实现模块**的生产区扫构造点，并要求
//! 每个 `Op` 变体在**登记的模块**里至少有一个**构造**（不是模式）。
//!
//! ## 怎么分辨"构造"与"模式"（机械规则，逐条给理由）
//!
//! 1. `matches!(x, Op::V { .. })` ⇒ 模式（回溯同一行找未被闭合的 `matches!(`）；
//! 2. 花括号配对之后的第一个非空白是 `=>` ⇒ 模式（match 臂）；
//! 3. `Op::V` 前面（跳过空白）是 `|` ⇒ 模式（或模式交替）、其余一律算构造。
//!
//! 规则②用**花括号配对**（不是看行尾），因此跨行的变体体也能判对。
//!
//! ## 为什么"登记的模块"是一张表而不是"任一模块"
//!
//! 只要求"某个模块有构造点"时，把 `import_audio` 里的构造点删掉**不会**红
//! （`section_build` 里还有一份）—— 那是"两条互为冗余的防线"同一类问题。
//! 本表逐**变体 × 模块**登记，因此删掉任何一处登记的构造点都会红。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yeban_mcp::domain::notes::OP_KINDS;
use yeban_mcp::undo_session::read_rust_sources;

/// 工具实现模块（相对 `crates/yeban-mcp/src`）：只在这些文件的生产区里扫构造点。
pub const TOOL_MODULES: [&str; 7] = [
    "domain/notes.rs",
    "domain/mod.rs",
    "domain/macros.rs",
    "domain/automation.rs",
    "domain/import_audio.rs",
    "domain/project_create.rs",
    "domain/section_build.rs",
];

/// 黄金表：每个 `Op` 变体**必须**在哪些工具模块里有一个构造点。
///
/// 读法：删掉任一处登记的构造点 ⇒ 本判据红。
pub const EXPECTED_SITES: [(&str, &[&str]); 31] = [
    ("AddNote", &["domain/notes.rs"]),
    ("DeleteNote", &["domain/notes.rs"]),
    ("MoveNote", &["domain/notes.rs"]),
    ("ModifyNoteVelocity", &["domain/notes.rs"]),
    (
        "AddClip",
        &[
            "domain/notes.rs",
            "domain/import_audio.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
    ("RemoveClip", &["domain/notes.rs"]),
    (
        "AddRoutingNode",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("RemoveRoutingNode", &["domain/notes.rs"]),
    (
        "AddClipPlacement",
        &[
            "domain/mod.rs",
            "domain/import_audio.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
    ("RemoveClipPlacement", &["domain/mod.rs"]),
    ("MoveClipPlacement", &["domain/mod.rs"]),
    (
        "AddTrack",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("RemoveTrack", &["domain/notes.rs"]),
    (
        "ConnectRouting",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("DisconnectRouting", &["domain/notes.rs"]),
    ("SetRoutingGain", &["domain/notes.rs"]),
    ("InsertDevice", &["domain/notes.rs"]),
    ("RemoveDevice", &["domain/notes.rs"]),
    ("SetParam", &["domain/notes.rs"]),
    ("SetTrackMute", &["domain/notes.rs"]),
    ("SetTrackSolo", &["domain/notes.rs"]),
    ("SetMacro", &["domain/macros.rs"]),
    (
        "SetAutomationPoint",
        &["domain/automation.rs", "domain/macros.rs"],
    ),
    ("RemoveAutomationPoint", &["domain/notes.rs"]),
    ("SetAutomationLane", &["domain/notes.rs"]),
    ("RemoveAutomationLane", &["domain/notes.rs"]),
    ("SetSection", &["domain/section_build.rs"]),
    ("RemoveSection", &["domain/notes.rs"]),
    ("SetScene", &["domain/notes.rs"]),
    ("RemoveScene", &["domain/notes.rs"]),
    (
        "Batch",
        &[
            "domain/mod.rs",
            "domain/macros.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 该文本里**被构造**的 `Op` 变体名（已剔除 match 模式）。
fn constructed_variants(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    let mut out = BTreeSet::new();
    let mut index = 0usize;
    while let Some(offset) = text[index..].find("Op::") {
        let at = index + offset;
        index = at + 4;
        // 排除 `NoteOp::`：前一个字符不得是标识符字符
        if at > 0 {
            let previous = bytes[at - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                continue;
            }
        }
        // 变体名
        let name_start = at + 4;
        let name_end = text[name_start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(text.len(), |skip| name_start + skip);
        let name = &text[name_start..name_end];
        if name.is_empty() {
            continue;
        }
        // 变体后面（跳过空白）必须是 `{`；否则是单元变体（`Op::RemoveClip` 之类）⇒ 构造
        let mut cursor = name_end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            out.insert(name.to_owned());
            continue;
        }
        // 花括号配对
        let mut depth = 0i32;
        let mut end = cursor;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let mut after = end + 1;
        while after < bytes.len() && bytes[after].is_ascii_whitespace() {
            after += 1;
        }
        // 规则②：紧跟 `=>` ⇒ match 臂模式
        if text[after..].starts_with("=>") {
            continue;
        }
        // 规则①：同一行里前面有未闭合的 `matches!(` ⇒ 模式
        let line_start = text[..at].rfind('\n').map_or(0, |position| position + 1);
        let prefix = &text[line_start..at];
        if let Some(call) = prefix.rfind("matches!(")
            && !prefix[call..].contains(')')
        {
            continue;
        }
        // 规则③：前面（跳过空白）是 `|` ⇒ 模式交替
        let trimmed = prefix.trim_end();
        if trimmed.ends_with('|') {
            continue;
        }
        out.insert(name.to_owned());
    }
    out
}

/// 该文本里 `parse_one` 的 match 臂认得的 `kind`（把常量名解析成字面量）。
fn kinds_in_parse_one(text: &str) -> BTreeSet<String> {
    // 先建常量表：`pub const X_KIND: &str = "literal";`
    let mut constants: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pub const ")
            && let Some((name, tail)) = rest.split_once(':')
            && let Some(value) = tail.split('"').nth(1)
        {
            constants.insert(name.trim().to_owned(), value.to_owned());
        }
    }
    let Some(start) = text.find("fn parse_one(") else {
        return BTreeSet::new();
    };
    let body = &text[start..];
    let Some(end) = body.find("\n}\n") else {
        return BTreeSet::new();
    };
    let body = &body[..end];
    let mut out = BTreeSet::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let Some((lhs, _rhs)) = trimmed.split_once("=>") else {
            continue;
        };
        let lhs = lhs.trim();
        if let Some(literal) = lhs
            .strip_prefix('"')
            .and_then(|rest| rest.split('"').next())
        {
            out.insert(literal.to_owned());
        } else if let Some(value) = constants.get(lhs) {
            out.insert(value.clone());
        }
    }
    out
}

fn sources() -> Vec<(String, String)> {
    read_rust_sources(&[manifest_dir().join("src")])
}

fn module_text(files: &[(String, String)], suffix: &str) -> Option<String> {
    files
        .iter()
        .find(|(path, _text)| path.replace('\\', "/").ends_with(suffix))
        .map(|(_path, text)| production_region(text))
}

#[test]
fn every_op_variant_has_a_registered_construction_site() {
    let files = sources();
    assert!(!files.is_empty(), "必须真的读到源码");
    let mut missing: Vec<String> = Vec::new();
    for (variant, modules) in EXPECTED_SITES {
        let mut found_any = false;
        for module in modules {
            let Some(text) = module_text(&files, module) else {
                missing.push(format!("{variant}: 读不到模块 {module}"));
                continue;
            };
            if constructed_variants(&text).contains(variant) {
                found_any = true;
            } else {
                missing.push(format!("{variant}: {module} 里没有构造点"));
            }
        }
        if !found_any {
            missing.push(format!("{variant}: 一个构造点都没有"));
        }
    }
    assert!(
        missing.is_empty(),
        "这些 (变体, 模块) 的构造点不见了（工具面因此可能不可达）: {missing:#?}"
    );
    // 登记表必须覆盖**恰好** 31 个变体，而且名字不重复。
    let names: BTreeSet<&str> = EXPECTED_SITES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), EXPECTED_SITES.len(), "变体名不得重复");
    assert_eq!(names.len(), 31, "模型 `Op` 有 31 个变体");
}

#[test]
fn the_catalog_kinds_are_exactly_the_parse_arms() {
    let files = sources();
    let notes = module_text(&files, "domain/notes.rs").expect("读得到 notes.rs");
    let arms = kinds_in_parse_one(&notes);
    let catalog: BTreeSet<String> = OP_KINDS.iter().map(|kind| (*kind).to_owned()).collect();
    let only_catalog: Vec<&String> = catalog.difference(&arms).collect();
    let only_arms: Vec<&String> = arms.difference(&catalog).collect();
    assert!(
        only_catalog.is_empty() && only_arms.is_empty(),
        "`OP_KINDS` 与 `parse_one` 的 match 臂**逐字**必须互为双射。\n\
         只在目录里（没有解析臂）: {only_catalog:?}\n只在解析臂里（没登记）: {only_arms:?}"
    );
}

/// 分类器**真的**分得开"构造"与"模式"（本判据喂纯合成的三行文本）。
///
/// 为什么必须有这一条：`every_op_variant_has_a_registered_construction_site` 的
/// 结论完全依赖 [`constructed_variants`]，而它的三条规则是从真实源码里"看出来"的。
/// 没有这一条时，把规则②（紧跟 `=>`）删掉，真实源码上**看不出差别**
/// （因为那些模式旁边还有别的判据兜着）—— 那是"冗余防线"的经典形状。
///
/// 注入（实测红）：删掉规则②（`if text[after..].starts_with("=>")`）、
/// 规则①（`matches!(`）或规则③（`|`）中的任一条 ⇒ 本判据红。
#[test]
fn the_classifier_tells_constructions_from_patterns() {
    // 构造：`push(...)`、`let x = ...`、`=> Op::V {`（右值是构造）。
    let constructions = concat!(
        "ops.push(Op::AddTrack { track: t });\n",
        "let op = Op::SetAutomationPoint { lane, tick, value, curve };\n",
        "Self::Mute => Op::SetTrackMute { track_id, old_mute, new_mute },\n",
        "Ok(Op::AddNote { track_id, clip_id, note })\n",
    );
    assert_eq!(
        constructed_variants(constructions),
        ["AddNote", "AddTrack", "SetAutomationPoint", "SetTrackMute"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    // 模式：match 臂 / `matches!` / 或模式交替 —— 一个都不许被当成构造。
    let patterns = concat!(
        "match op { Op::AddClip { clip } => { let _ = clip; } _ => {} }\n",
        "if matches!(op, Op::AddClip { .. }) { return; }\n",
        "matches!(op, Op::AddNote { .. } | Op::DeleteNote { .. });\n",
    );
    assert!(
        constructed_variants(patterns).is_empty(),
        "模式不得被当成构造: {:?}",
        constructed_variants(patterns)
    );
    // `NoteOp::` 前缀不算（本 crate 的两个枚举前缀重叠，这是最容易错的字面陷阱）。
    assert!(
        constructed_variants("let x = NoteOp::AddNote { track_id, clip_id, note };").is_empty()
    );
    // 单元变体（后面不跟 `{`）算构造。
    assert!(constructed_variants("let x = Op::RemoveClip;").contains("RemoveClip"));
}
