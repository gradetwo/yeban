//! 九语 i18n 覆盖判据（line/ui-i18n，2026-11-17 扩九语）。
//!
//! 产品口径（Wang 2026-11-17 定，与偃师 web 端 i18n 对齐）：界面文案走单一
//! 词表、按中文原文键、默认中文、其余八语显式切换、**漏一条当场红**。本文件是
//! 这条口径在 Slint 侧的机械形态，共三层，各自能独立变红：
//!
//! | # | 判据 | 观测面 |
//! | :-- | :--- | :--- |
//! | ① | `ui/**/*.slint` 里任何含汉字的字符串字面量，只要不是 `@tr(...)` 的直接参数、也不在白名单内 ⇒ 红（报文件:行号 + 原文） | 全部 `.slint` 源码文本 |
//! | ② | 源码 `@tr` 的 msgid 集合与**全部八个目标语言**词表的 msgid 集合**精确相等**（按解析值比，转义两侧同规），且每条 `msgstr` 非空；缺条 / 孤儿条 / 空译都红。另逐语做删词条变异：每语删 `选区 无` 一条，判据必须当场点名该条（变异可抽样，全量对账在判据本体） | `.slint` 源码 + 八份 `.po` 文本 |
//! | ③ | 判据自身的合成变异自检：喂一段带假硬编码中文的合成源码，①必须抓住；同一段改走 `@tr` 后必须放行 | 本文件的检查函数 |
//!
//! ## 白名单（逐条带理由，只许按值精确匹配）
//!
//! - `C Minor (自然小调)` / `D Dorian (多利亚)` / `F Major (大调)`：调式
//!   **规范值**。走带栏的循环逻辑按中文原文比较与赋值（`scale-text` 属性），
//!   宿主与工程数据也按此值流动；只在显示点（transport_bar.slint 的 LCD
//!   Text）用 `@tr` 映射这三个值，未知值原样显示。把规范值本身包上 `@tr`
//!   会让逻辑比较在英文态下失配，所以留白。
//! - `夜半`：产品品牌名本体（专有名词不译；品牌标记的 tooltip）。
//! - `中文` / `日本語`：语言自称名——主菜单「设置 → 语言」里的语言名
//!   （中文态显示「中文」、日语态显示「日本語」），自称名按定义不翻译。
//!   其余七语自称名不含汉字，判据天然碰不到，不进白名单。
//!   （走带栏语言 chip 已按负责人 2026-11-17 口径撤销，菜单是唯一入口。）
//!
//! ## 不在本判据范围内的（已在 docs/app-i18n-notes.md 登记）
//!
//! - Rust 侧宿主写入的瞬时状态/提示文本（toast、事件回执等诊断类文案）：
//!   照偃师口径日志与诊断不翻译，本期不覆盖。例外是宿主写入的**闭集常驻
//!   显示标签**：总览面板曲风档名（由 `song_overview::profile_display_name_for`
//!   按语言取名）与状态栏选区 / 音频配置两格（`host::publish_status_bar`
//!   按语言取词、`host::apply_language` 切换时重标）——状态栏是常驻 UI
//!   显示、不是日志，2026-11-17 收尾裁定须译。第四处是模板选择器的来源
//!   分组标题（`templates::TemplateSource::group_label_for`，行模型打开
//!   选择器时按当前语言构建）。四处都不在本判据的 .slint 观测面内，各自
//!   由九语单测看守（`src/song_overview.rs` / `src/host.rs` /
//!   `src/templates.rs` 内）。
//! - 用户内容（轨道名、片段名、工程数据）：不是源码字面量，本判据天然
//!   碰不到，也不许碰。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// 白名单：见文件头注释逐条理由。只许按**解析后的值**精确匹配。
const WHITELIST: &[&str] = &[
    "C Minor (自然小调)",
    "D Dorian (多利亚)",
    "F Major (大调)",
    "夜半",
    "中文",
    "日本語",
];

/// 全部八个目标语言的词表目录名（源语言 zh-CN 没有词表目录，不在内）。
const TARGET_LOCALES: &[&str] = &[
    "en-US", "fr-FR", "es-ES", "pt-BR", "ja-JP", "ko-KR", "ru-RU", "de-DE",
];

fn is_cjk(ch: char) -> bool {
    ('\u{3400}'..='\u{4dbf}').contains(&ch) || ('\u{4e00}'..='\u{9fff}').contains(&ch)
}

/// 一个字符串字面量的扫描结果（值已去转义）。
struct Literal {
    value: String,
    line: usize,
    /// 是否是 `@tr(` 的直接第一参数。
    is_tr_arg: bool,
}

/// 把 Slint / PO 共用的转义序列还原成值：`\\` `\"` `\n` `\t` `\{`。
fn unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// 词法扫描一份 .slint 源码：跳过 `//` 与 `/* */` 注释，逐个产出字符串字面量。
fn scan_slint(source: &str) -> Vec<Literal> {
    let bytes: Vec<char> = source.chars().collect();
    let mut literals = Vec::new();
    let mut i = 0usize;
    let mut line = 1usize;
    while i < bytes.len() {
        let ch = bytes[i];
        if ch == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if ch == '/' && i + 1 < bytes.len() && bytes[i + 1] == '/' {
            while i < bytes.len() && bytes[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if ch == '/' && i + 1 < bytes.len() && bytes[i + 1] == '*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == '*' && bytes[i + 1] == '/') {
                if bytes[i] == '\n' {
                    line += 1;
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if ch == '"' {
            let start_line = line;
            // 直接参数判定：字面量之前（跳过空白）以 `@tr(` 结尾。
            let before: String = bytes[..i].iter().collect();
            let is_tr_arg = before.trim_end().ends_with("@tr(");
            let mut raw = String::new();
            i += 1;
            while i < bytes.len() {
                match bytes[i] {
                    '\\' if i + 1 < bytes.len() => {
                        raw.push(bytes[i]);
                        raw.push(bytes[i + 1]);
                        i += 2;
                    }
                    '"' => {
                        i += 1;
                        break;
                    }
                    c => {
                        if c == '\n' {
                            line += 1;
                        }
                        raw.push(c);
                        i += 1;
                    }
                }
            }
            literals.push(Literal {
                value: unescape(&raw),
                line: start_line,
                is_tr_arg,
            });
            continue;
        }
        i += 1;
    }
    literals
}

/// 判据 ①：未路由的硬编码中文（返回违规清单，空 = 通过）。
fn unrouted_violations(files: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for (name, source) in files {
        for lit in scan_slint(source) {
            if !lit.value.chars().any(is_cjk) {
                continue;
            }
            if lit.is_tr_arg || WHITELIST.contains(&lit.value.as_str()) {
                continue;
            }
            violations.push(format!("{name}:{}: {}", lit.line, lit.value));
        }
    }
    violations
}

/// 源码侧 `@tr` msgid 集合（解析值）。
fn source_msgids(files: &[(String, String)]) -> BTreeSet<String> {
    let mut ids = BTreeSet::new();
    for (_, source) in files {
        for lit in scan_slint(source) {
            if lit.is_tr_arg {
                ids.insert(lit.value);
            }
        }
    }
    ids
}

/// 解析 .po 文本：返回 (msgid 集合, 空 msgstr 的 msgid 清单)。
/// 只认本仓词表的简单形态：`msgid "..."` / `msgstr "..."` 加 `"..."` 续行。
fn parse_po(po: &str) -> (BTreeSet<String>, Vec<String>) {
    let mut ids = BTreeSet::new();
    let mut empty = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_str: Option<String> = None;
    let mut mode: Option<&str> = None;
    let mut flush = |id: &mut Option<String>, s: &mut Option<String>| {
        if let (Some(id), Some(s)) = (id.take(), s.take())
            && !id.is_empty()
        {
            if s.is_empty() {
                empty.push(id.clone());
            }
            ids.insert(id);
        }
    };
    for raw_line in po.lines() {
        let line = raw_line.trim();
        if let Some(rest) = line.strip_prefix("msgid ") {
            flush(&mut current_id, &mut current_str);
            current_id = Some(po_string_value(rest));
            mode = Some("id");
        } else if let Some(rest) = line.strip_prefix("msgstr ") {
            current_str = Some(po_string_value(rest));
            mode = Some("str");
        } else if line.starts_with('"') {
            let value = po_string_value(line);
            match mode {
                Some("id") => {
                    if let Some(id) = &mut current_id {
                        id.push_str(&value);
                    }
                }
                Some("str") => {
                    if let Some(s) = &mut current_str {
                        s.push_str(&value);
                    }
                }
                _ => {}
            }
        } else if line.is_empty() || line.starts_with('#') {
            mode = None;
        }
    }
    flush(&mut current_id, &mut current_str);
    (ids, empty)
}

/// 取一行 PO 字符串字面量（`"..."`）的解析值。
fn po_string_value(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        unescape(&text[1..text.len() - 1])
    } else {
        String::new()
    }
}

/// 递归收集 `ui/` 下全部 .slint 文件（相对路径名, 内容）。
fn slint_files() -> Vec<(String, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut files = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap_or_else(|e| panic!("读 ui 目录失败 {}: {e}", dir.display()))
            .map(|e| e.expect("ui 目录项").path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "slint") {
                let name = path
                    .strip_prefix(&root)
                    .expect("ui 子路径")
                    .to_string_lossy()
                    .replace('\\', "/");
                let content = std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()));
                files.push((format!("ui/{name}"), content));
            }
        }
    }
    files.sort();
    files
}

fn po_source() -> String {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("translations/en-US/LC_MESSAGES/yeban-app.po");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()))
}

#[test]
fn no_unrouted_chinese_literals() {
    let violations = unrouted_violations(&slint_files());
    assert!(
        violations.is_empty(),
        "未走 @tr 的硬编码中文字符串（漏一条即红，口径见本文件头）:\n{}",
        violations.join("\n")
    );
}

#[test]
fn catalog_matches_source_msgids() {
    let files = slint_files();
    let src_ids = source_msgids(&files);
    let (po_ids, empty) = parse_po(&po_source());
    let missing: Vec<&String> = src_ids.difference(&po_ids).collect();
    let orphans: Vec<&String> = po_ids.difference(&src_ids).collect();
    assert!(
        missing.is_empty(),
        "en-US.po 缺条（源码有 @tr、词表没有）:\n{}",
        missing
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        orphans.is_empty(),
        "en-US.po 孤儿条（词表有、源码已不用）:\n{}",
        orphans
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(
        empty.is_empty(),
        "en-US.po 空译条（msgstr 为空）:\n{}",
        empty.join("\n")
    );
}

fn po_source_for_locale(locale: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("translations/{locale}/LC_MESSAGES/yeban-app.po"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("读 {} 失败: {e}", path.display()))
}

/// 从 .po 文本里删掉一条 msgid 的整个条目（msgid 行起，至空行前为止），
/// 供变异证据用：删后重解析，判据必须当场点名被删的那条。
fn remove_po_entry(po: &str, msgid: &str) -> String {
    let target = format!("msgid \"{msgid}\"");
    let mut lines: Vec<&str> = po.lines().collect();
    let Some(start) = lines.iter().position(|line| line.trim() == target) else {
        panic!("变异目标 {msgid} 在词表里不存在，变异做不成");
    };
    let mut end = start + 1;
    while end < lines.len() && !lines[end].trim().is_empty() {
        end += 1;
    }
    lines.drain(start..end);
    lines.join("\n")
}

/// 判据 ② 的八语版：八份词表逐一与源码 `@tr` 集合精确对账（缺条 /
/// 孤儿条 / 空译都红），并逐语做删词条变异（删 `选区 无` 一条，判据
/// 必须当场点名该条；变异可抽样，全量对账在判据本体）。
#[test]
fn all_locales_match_source_msgids_with_mutation_evidence() {
    let files = slint_files();
    let src_ids = source_msgids(&files);
    for locale in TARGET_LOCALES {
        let po = po_source_for_locale(locale);
        let (po_ids, empty) = parse_po(&po);
        let missing: Vec<&String> = src_ids.difference(&po_ids).collect();
        let orphans: Vec<&String> = po_ids.difference(&src_ids).collect();
        assert!(
            missing.is_empty(),
            "{locale} 词表缺条（源码有 @tr、词表没有）:\n{}",
            missing
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            orphans.is_empty(),
            "{locale} 词表孤儿条（词表有、源码已不用）:\n{}",
            orphans
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("\n")
        );
        assert!(
            empty.is_empty(),
            "{locale} 词表空译条（msgstr 为空）:\n{}",
            empty.join("\n")
        );
        // 变异证据：删 `选区 无` 一条，判据必须只报这一条、且点名它。
        let mutated = remove_po_entry(&po, "选区 无");
        let (mutated_ids, _) = parse_po(&mutated);
        let caught: Vec<&String> = src_ids.difference(&mutated_ids).collect();
        assert_eq!(
            caught.len(),
            1,
            "{locale} 词表删 `选区 无` 一条后，判据必须只报这一条（实报: {caught:?})"
        );
        assert_eq!(
            caught[0].as_str(),
            "选区 无",
            "{locale} 词表删条变异必须当场点名被删的 `选区 无`"
        );
    }
}

#[test]
fn checker_catches_synthetic_violations() {
    // 变异自检（判据 ③）：假硬编码必须被抓、改走 @tr 必须放行、白名单必须放行。
    let bad = vec![(
        "ui/fake.slint".to_owned(),
        "Text { text: \"变异假字符串\"; }".to_owned(),
    )];
    assert_eq!(unrouted_violations(&bad).len(), 1);
    let good = vec![(
        "ui/fake.slint".to_owned(),
        "Text { text: @tr(\"变异假字符串\"); }".to_owned(),
    )];
    assert!(unrouted_violations(&good).is_empty());
    let whitelisted = vec![(
        "ui/fake.slint".to_owned(),
        "Text { text: \"夜半\"; }".to_owned(),
    )];
    assert!(unrouted_violations(&whitelisted).is_empty());
    // 注释里的中文不是字符串，不许误报。
    let comment_only = vec![(
        "ui/fake.slint".to_owned(),
        "// 注释里的中文\nText { text: \"OK\"; }".to_owned(),
    )];
    assert!(unrouted_violations(&comment_only).is_empty());
}
