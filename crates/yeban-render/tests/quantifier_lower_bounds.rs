//! **常驻判据（R115）**: 本 crate 的判据里, 凡在 `assert!` 里出现**量词**（`.all(`, `.any(`, `.windows(`）
//! 的地方, 都必须在**同一个根表达式**上有"界"。
//!
//! # 为什么要有它（三条裁决的落地）
//!
//! - **R102**: `.all(..)` 在**空集合**上恒真; `.any(..)` 在空集合上恒假 ⇒ **取反后同样恒真**
//!   ⇒ 量词断言在空集合上会**真空通过**。
//! - **R111**: "界"有**五种形态** —— ①显式 `x.len() >= N` ②宏隐式相等 `assert_eq!(x.len(), N)`
//!   ③`!x.is_empty()` ④值界（`match x { .. }` 或 `x == N` 这类把取值钉住的断言）
//!   ⑤**值界** `x.len() == N`。⚠ **R118**: **运行期计数器不算** —— 它数迭代次数, 不界定被遍历的集合。
//! - **R114**: 界必须**根绑定** —— 同一个根表达式的界, ⛔ 不许"借用邻居"（别的集合有界不算）。
//! - **R113**: 静态扫描的掩码必须**逐字节等长**（按 `len_utf8()` 补空格）并**保留换行**,
//!   否则行号/偏移会漂移。
//!
//! # 判据自带 R56 对照（⛔ 不是"跑一次就算"）
//!
//! `checker_has_teeth` 用**合成片段**把检查器喂一遍: 五种界形态各喂一条**已知绿**,
//! 再喂"无界的量词"**已知红**, 以及含**多字节注释**的片段（R113）。
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// R113: 把注释与字符串字面量掩成空格, **逐字节等长**（多字节字符按其 `len_utf8()` 补空格）,
/// 并**保留换行**（行号可用）。
fn mask(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < text.len() {
        let rest = &text[i..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(text.len(), |offset| i + offset);
            mask_span(&mut out, &text[i..end]);
            i = end;
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(text.len(), |offset| i + offset + 2);
            mask_span(&mut out, &text[i..end]);
            i = end;
        } else if let Some((offset_in_rest, closer)) = raw_string_head(rest) {
            // ⚠ `raw_string_head` 给的是**相对 `rest` 的**偏移 ⇒ 必须加上 `i`（本机实测的越界 bug）。
            let body = i + offset_in_rest;
            let end = text[body..]
                .find(&closer)
                .map_or(text.len(), |offset| body + offset + closer.len());
            mask_span(&mut out, &text[i..end]);
            i = end;
        } else if let Some(len) = char_literal_len(rest) {
            mask_span(&mut out, &text[i..i + len]);
            i += len;
        } else if rest.starts_with('"') {
            let bytes = text.as_bytes();
            let mut j = i + 1;
            while j < bytes.len() {
                if bytes[j] == b'\\' {
                    j += 2;
                    continue;
                }
                if bytes[j] == b'"' {
                    break;
                }
                j += 1;
            }
            let end = (j + 1).min(text.len());
            mask_span(&mut out, &text[i..end]);
            i = end;
        } else {
            let ch = rest.chars().next().expect("非空");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// 把 `slice` **按字节等长**地抹成空格（换行保留）—— 掩码的唯一定长手段（R113）。
fn mask_span(out: &mut String, slice: &str) {
    for ch in slice.chars() {
        if ch == '\n' {
            out.push('\n');
        } else {
            out.push_str(&" ".repeat(ch.len_utf8()));
        }
    }
}

/// 原始字符串头: `r"` / `r#"` / `br#"` … ⇒ 返回（正文起点, 结束定界符）。
fn raw_string_head(rest: &str) -> Option<(usize, String)> {
    let prefix = if rest.starts_with("br") {
        2
    } else if rest.starts_with('r') {
        1
    } else {
        return None;
    };
    let after = &rest[prefix..];
    let hashes = after.chars().take_while(|c| *c == '#').count();
    if !after[hashes..].starts_with('"') {
        return None;
    }
    Some((prefix + hashes + 1, format!("\"{}", "#".repeat(hashes))))
}

/// **字符字面量**的字节长度（`'x'` / `'\n'` / `'\''`）; 生命周期 `'a` ⇒ `None`（⛔ 不许抹到下一个 `'`）。
fn char_literal_len(rest: &str) -> Option<usize> {
    if !rest.starts_with('\'') {
        return None;
    }
    let mut chars = rest.chars();
    chars.next()?;
    let second = chars.next()?;
    if second == '\\' {
        let mut len = 2;
        for ch in chars {
            len += ch.len_utf8();
            if ch == '\'' {
                return Some(len);
            }
        }
        None
    } else {
        let third = chars.next()?;
        if third == '\'' {
            Some(1 + second.len_utf8() + 1)
        } else {
            None
        }
    }
}

/// 取一个表达式片段的**根**: 从**紧邻量词之前**往回读标识符链（而不是从行首往前读 ——
/// 那样会把 `header.len() >= 4 && !header` 的根读成 `header.len`）, 再剥掉视图链后缀
/// （`.iter`, `.values`, `.as_slice`, `.copied`, `.windows`, `.to_le_bytes`, …）。
fn root_of(prefix: &str) -> String {
    let mut trimmed = prefix.trim_end().to_owned();
    loop {
        let before = trimmed.clone();
        // (a) 剥掉尾部的、配对的调用括号: `x.iter()` ⇒ `x.iter`
        while trimmed.ends_with(')') {
            let mut depth = 0i32;
            let mut cut = None;
            for (index, ch) in trimmed.char_indices().rev() {
                match ch {
                    ')' => depth += 1,
                    '(' => {
                        depth -= 1;
                        if depth == 0 {
                            cut = Some(index);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            match cut {
                Some(index) => trimmed = trimmed[..index].trim_end().to_owned(),
                None => break,
            }
        }
        // (b) 剥掉视图链后缀
        for suffix in [
            ".iter",
            ".iter_mut",
            ".values",
            ".values_mut",
            ".as_slice",
            ".as_bytes",
            ".copied",
            ".flatten",
            ".windows",
            ".to_le_bytes",
            ".to_be_bytes",
            ".chunks",
            ".chunks_exact",
            ".chars",
            ".skip",
            ".filter",
            ".filter_map",
            ".map",
            ".find",
            ".find_map",
            ".take",
            ".rev",
            ".enumerate",
            ".zip",
        ] {
            if let Some(head) = trimmed.strip_suffix(suffix) {
                trimmed = head.trim_end_matches('.').trim_end().to_owned();
            }
        }
        if trimmed == before {
            break; // 交替到不动点
        }
    }
    let mut start = trimmed.len();
    for (index, ch) in trimmed.char_indices().rev() {
        if ch.is_alphanumeric() || ch == '_' || ch == '.' || ch.is_whitespace() {
            start = index;
        } else {
            break;
        }
    }
    let compact = trimmed[start..]
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<String>();
    compact.trim_matches('.').to_owned()
}

/// 把 `assert!` / `assert_eq!` 的实参按顶层逗号切开（掩码后的文本 ⇒ 字符串里的逗号不会干扰）。
fn split_top_level(args: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for ch in args.chars() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if ch == ',' && depth == 0 {
            parts.push(current.trim().to_owned());
            current.clear();
        } else {
            current.push(ch);
        }
    }
    if !current.trim().is_empty() {
        parts.push(current.trim().to_owned());
    }
    parts
}

/// 量词站点的根（`.all(` / `.any(` / `.windows(`）。
fn quantifier_roots(condition: &str) -> Vec<(String, bool)> {
    let mut roots = Vec::new();
    for needle in [".all(", ".any(", ".windows("] {
        let mut from = 0usize;
        while let Some(found) = condition[from..].find(needle) {
            let at = from + found;
            // 嵌套判定: 量词**之前**的括号深度 > 0 ⇒ 它落在某个调用/闭包的**实参里**
            // （如 `file.windows(190).any(|w| w.iter().all(..))` 的内层）⇒ 由外层量词负责。
            // ⚠ 用"`|` 数奇偶"是**错的**: 闭包的两个竖线都在内层量词之前 ⇒ 偶 = 漏判（本机实测）。
            let mut depth = 0i32;
            for ch in condition[..at].chars() {
                match ch {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
            }
            let nested = depth > 0;
            roots.push((root_of(&condition[..at]), nested));
            from = at + needle.len();
        }
    }
    roots
}

/// 去掉**全部空白**（空格/制表/换行）—— 跨行形态识别的前提。
fn strip_ws(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_whitespace()).collect()
}

/// **R119**: 带标识符边界的子串匹配 —— `needle` 之前的那个字符不得是标识符字符
/// （否则根 `b` 会被 `assert_eq!(bb.len(), 8)` 满足：near-miss）。`body` 已是**去空白**的文本。
fn contains_identifier(body: &str, needle: &str) -> bool {
    let mut from = 0usize;
    while let Some(found) = body[from..].find(needle) {
        let at = from + found;
        let ok = at == 0
            || !body[..at]
                .chars()
                .next_back()
                .is_some_and(|ch| ch.is_alphanumeric() || ch == '_');
        if ok {
            return true;
        }
        from = at + 1;
    }
    false
}

/// 五种界形态之一是否落在**同一个根**上（R111 ＋ R114）。
/// 形态 → 该形态可用的**针**（`{root}` 会被替换）。路径: `condition` = 断言表达式里, `body` = 同函数体里。
///
/// **R185**: 分类器本身就是判据 ⇒ 每个形态必须**可单独归因**（哪个形态把这条断言判成"有界"）。
fn form_needles(form: &str, root: &str) -> Vec<(&'static str, String)> {
    match form {
        "explicit-len-ge" => vec![
            ("condition", format!("{root}.len() >=")),
            ("condition", format!("{root}.len() >")),
            ("body", format!("assert!({root}.len()>=")),
            ("body", format!("assert!({root}.len()>")),
        ],
        "macro-implicit-len-eq" => vec![
            ("body", format!("assert_eq!({root}.len(),")),
            ("body", format!("assert_eq!({root}.len(),16")),
        ],
        "not-is-empty" => vec![
            ("condition", format!("!{root}.is_empty()")),
            ("condition", format!("!{root}.is_empty() &&")),
            ("condition", format!("{root}.is_empty() ==")),
            ("body", format!("assert!(!{root}.is_empty()")),
        ],
        "value-bound-len-eq" => vec![
            ("condition", format!("{root}.len() ==")),
            ("condition", format!("{root}.len()==")),
        ],
        // R125: `assert!(x.is_empty(), …)` —— "空表是**有意**的"（对照夹具）⇒ 同样算把域钉住。
        "explicit-empty-table" => vec![
            ("condition", format!("assert!({root}.is_empty()")),
            ("body", format!("assert!({root}.is_empty()")),
        ],
        other => unreachable!("未注册的形态: {other}"),
    }
}

/// 只启用 `enabled` 里的形态, 返回**第一个命中**的形态名（`None` = 无界）。
///
/// **R119**: 子串匹配必须带**标识符边界**（否则根 `b` 会被 `ab.len()` 满足 —— near-miss）。
/// **R139**: 去**全部空白**（只去空格会漏掉**跨行形态**, 那是假阳性）。
fn bound_form_for<'a>(
    enabled: &[&'a str],
    function_body: &str,
    condition: &str,
    root: &str,
) -> Option<&'a str> {
    if root.is_empty() {
        return Some("root-not-recovered"); // 取不出根（字面量等）⇒ 不判, 交给人工
    }
    let cond = strip_ws(condition);
    let body = strip_ws(function_body);
    for form in enabled {
        for (path, needle) in form_needles(form, root) {
            let hay = if path == "condition" { &cond } else { &body };
            if contains_identifier(hay, &strip_ws(&needle)) {
                return Some(form);
            }
        }
    }
    None
}

/// 扫一个源文件, 返回"没有根绑定下界的量词站点"（行号 ＋ 根）。
fn unbounded_quantifiers(source: &str) -> Vec<(usize, String)> {
    unbounded_quantifiers_with(&REGISTERED_BOUND_FORMS, source)
}

/// 只把 `enabled` 里的形态当作"界"来扫描（`enabled = &[]` ⇒ 每个站点都算无界 ⇒ 总站点数）。
fn unbounded_quantifiers_with(enabled: &[&str], source: &str) -> Vec<(usize, String)> {
    let masked = mask(source);
    assert_eq!(masked.len(), source.len(), "R113: 掩码必须逐字节等长");
    let mut offenders = Vec::new();
    // 粗切函数体: 以 `    fn ` 为界
    let mut functions: BTreeMap<usize, String> = BTreeMap::new();
    let mut starts: Vec<usize> = masked
        .match_indices("\n    fn ")
        .map(|(index, _)| index)
        .collect();
    starts.push(masked.len());
    for window in starts.windows(2) {
        functions.insert(window[0], masked[window[0]..window[1]].to_owned());
    }
    let mut from = 0usize;
    while let Some(found) = masked[from..].find("assert!(") {
        let at = from + found;
        let start = at + "assert!(".len();
        let mut depth = 1i32;
        let mut end = start;
        for (offset, ch) in masked[start..].char_indices() {
            match ch {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        end = start + offset;
                        break;
                    }
                }
                _ => {}
            }
        }
        let args = &masked[start..end];
        if let Some(condition) = split_top_level(args).first() {
            // 嵌套量词（写在闭包体里）由外层量词负责 ⇒ 只判**顶层**量词。
            let roots = quantifier_roots(condition)
                .into_iter()
                .filter(|(_, nested)| !nested)
                .map(|(root, _)| root)
                .collect::<Vec<_>>();
            if !roots.is_empty() {
                let line = source[..at].matches('\n').count() + 1;
                let body = functions
                    .range(..=at)
                    .next_back()
                    .map(|(_, body)| body.clone())
                    .unwrap_or_default();
                for root in roots {
                    if bound_form_for(enabled, &body, condition, &root).is_none() {
                        offenders.push((line, root));
                    }
                }
            }
        }
        from = end;
    }
    offenders
}

/// 判据: 本 crate 的**所有源文件**里都不许有"无根绑定下界的量词断言"。
#[test]
fn no_unbounded_quantifier_assertion_in_this_crate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = std::fs::read_dir(root.join("src"))
        .expect("读 src/")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    // **R196 的实验读数（[R187-PROBE render/tests/quantifier_lower_bounds R196-FLOORS]）**: 取消"至少 7 个源文件"这一行,
    // **受害清单为空** ⇒ 这个约束**从未提供过证据**（真正的下界在 `scan_reached` 里, 那条有受害清单）
    // ⇒ 按 R196 **降级为诊断**（打印, 不判红）。⛔ 这不是说"放宽更好"。
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R196-FLOORS] src 文件数 = {}（诊断, 非断言; 下界由 scan_reached 承担）",
        files.len()
    );
    let mut scanned = 0usize;
    // R132: 入口表**保留但恒空**（入口数 = 未修缺口数 = 0）—— 双向相等断言仍在下文。
    let skipped: Vec<String> = Vec::new();
    let mut offenders = Vec::new();
    for path in &files {
        // **R132（入口数 = 未修缺口数）**: 第二十一批把检查器的**根因**修好了
        // （`replace(' ', "")` 不去**换行** ⇒ 跨行形态的界识别不到 ⇒ **假阳性**;
        // 修好后, 第二十批那 11 处里 **3 处（rich ×5 与 empty ×1 与助手 ×2 同函数）当场被认出**,
        // 剩下 **8 处**是**真缺口**, 位置已在报告里逐条列出:
        // `logic.rs:5959/5966/5973 bundle.losses`、`6377 filter`、
        // `6712/6718 data.losses`、`8276 two.losses`、`8293 data.losses`。
        // ⇒ 在补完这 8 处之前, 这 2 个文件**继续跳过**（入口数 = 2, 与上一批相同 ——
        //    ⛔ 不增, 但**未修缺口数从 11 降到 8**, 这是本批的进度读数）。
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        // **R132**: 入口已**全部撤回**（2 → 0）—— 8 处真缺口已逐处按 R125/R131 处置,
        // 且夹具的域都经**实测**确认非空（feature 档 265 条全绿即证）。
        let _ = &name;
        let source = std::fs::read_to_string(path).expect("读源文件");
        scanned += 1;
        for (line, root_name) in unbounded_quantifiers(&source) {
            offenders.push(format!(
                "{}:{line} 根 `{root_name}`",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
        }
    }
    // **R119/R120 的机械下界**（助手有配对的已知红: `scan_lower_bounds_reject_broken_inputs`）
    scan_reached(files.len(), scanned, skipped.len())
        .expect("扫描必须覆盖全部文件、且没有跳过入口");
    assert!(
        offenders.is_empty(),
        "R102/R111/R114: 下列量词断言没有**根绑定**的下界（空集合上会真空通过）:\n{}",
        offenders.join("\n")
    );
}

/// 判据: 检查器自带 **R56 对照** —— 五种界形态各一条**已知绿** ＋ 两条**已知红**
/// （无界／借用邻居）＋ 多字节注释（R113）。用**原始字符串**写片段, 避免转义走样。
#[test]
fn checker_has_teeth() {
    // ① 显式下界（写在同一表达式里）
    assert!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(v.len() >= 8 && v.iter().all(|x| *x == 0));
    }"#
        )
        .is_empty(),
        "①显式下界必须被认"
    );
    // ② 宏隐式相等（同一个根）
    assert!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert_eq!(v.len(), 16);
        assert!(v.iter().all(|x| *x == 0));
    }"#
        )
        .is_empty(),
        "②宏隐式相等必须被认"
    );
    // ③ `!is_empty()` 写成独立的断言（同一个根）
    assert!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(!v.is_empty(), "非空");
        assert!(v.iter().any(|x| *x == 1));
    }"#
        )
        .is_empty(),
        "③is_empty 必须被认"
    );
    // 已知红（R118）: **只有运行期计数器** ⇒ 被遍历的集合仍未被界定 ⇒ 必须被抓住
    assert_eq!(
        unbounded_quantifiers(
            r#"
    fn t() {
        let mut count = 0;
        for x in v { count += 1; }
        assert_eq!(count, 4);
        assert!(v.iter().all(|y| *y == 0));
    }"#
        ),
        vec![(6usize, "v".to_owned())],
        "R118: 计数器数的是迭代次数, 不界定被遍历的集合"
    );
    // 已知红（R119, 条件内 near-miss）: `ab.len()` 不得被当成根 `b` 的界
    assert_eq!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(ab.len() >= 4 && b.iter().all(|x| *x == 0));
    }"#
        ),
        vec![(3usize, "b".to_owned())],
        "R119: 条件内的子串匹配也必须带标识符边界"
    );
    // 已知绿（R125）: **显式断言空表** —— 空转是"有意"的 ⇒ 不算无界
    assert!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(v.is_empty(), "对照夹具: 本来就该为空");
        assert!(!v.iter().any(|x| *x == 1));
    }"#
        )
        .is_empty(),
        "R125: 显式断言空表必须被认"
    );
    // 已知绿（R126: **跨行形态**定标）: 下界写在**多行** `assert!(` 里也必须被认
    assert!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(
            !v.is_empty(),
            "跨行形态"
        );
        assert!(
            v
                .iter()
                .all(|x| *x == 0),
            "跨行量词"
        );
    }"#
        )
        .is_empty(),
        "R126: 跨行形态的下界必须被认（本批修掉的假阳性）"
    );
    // 已知红（R133: **宏名**路径）: `my_assert!(!v.is_empty(), …)` 不是 `assert!` ⇒ 不算界
    assert_eq!(
        unbounded_quantifiers(
            r#"
    fn t() {
        my_assert!(!v.is_empty(), "另一个宏");
        assert!(v.iter().all(|x| *x == 0));
    }"#
        ),
        vec![(4usize, "v".to_owned())],
        "R133: 必须是 `assert!`, 不能被 `my_assert!` 满足"
    );
    // 已知红 ①: 完全没有界
    assert_eq!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert!(left.iter().all(|x| *x == 0.0));
    }"#
        ),
        vec![(3usize, "left".to_owned())],
        "无界量词必须被抓住（否则本判据没有牙）"
    );
    // 已知红 ②: **借用邻居** —— 邻居有界、本集合没有（R114）
    assert_eq!(
        unbounded_quantifiers(
            r#"
    fn t() {
        assert_eq!(right.len(), 8);
        assert!(left.iter().all(|x| *x == 0.0));
    }"#
        ),
        vec![(4usize, "left".to_owned())],
        "R114: 不许借用邻居的界"
    );
    // R113: 多字节注释与字符串必须**逐字节等长**地被掩掉, 且界仍被认
    let multibyte = "// 中文注释
    fn t() {
        let s = \"中文\";
        assert!(v.len() >= 1 && v.iter().all(|x| *x == 0), \"消息\");
    }";
    let masked = mask(multibyte);
    assert_eq!(masked.len(), multibyte.len(), "R113: 逐字节等长");
    assert_eq!(
        masked.matches('\n').count(),
        multibyte.matches('\n').count(),
        "换行保留"
    );
    assert!(
        unbounded_quantifiers(multibyte).is_empty(),
        "多字节片段里的界必须被认"
    );
}

/// **R122**: 三条常驻判据各自的**已知红**都要有一条**常驻对照**（⛔ 不能只靠历史批次的红）。
/// 本函数把"掩码器被写坏"这一已知红喂给长度校验, 并确认它会红。
#[test]
fn masking_check_reddens_when_the_masker_drops_bytes() {
    /// 故意写坏的掩码器: 注释整段删掉（**不补空格** ⇒ 字节数变短）。
    fn broken_mask(text: &str) -> String {
        text.lines()
            .map(|line| line.split("//").next().unwrap_or(line))
            .collect::<Vec<_>>()
            .join(
                "
",
            )
    }
    let sample = "// 注释\n    fn t() { }";
    assert_eq!(mask(sample).len(), sample.len(), "正确掩码: 等长");
    assert_ne!(
        broken_mask(sample).len(),
        sample.len(),
        "R122: 写坏的掩码器**必须**被长度校验抓住（否则本判据没有牙）"
    );
}

/// **R118/R119 的常驻对照**: 元素值界不算集合界; 前缀相近的兄弟根不算本根的界。
#[test]
fn value_and_near_miss_bounds_are_not_accepted() {
    // 已知红（R118）: 只有元素值界 ⇒ 集合可能为空 ⇒ 必须被抓住
    let element_only = r#"
    fn t() {
        assert_eq!(v[0], 5);
        assert!(v.iter().all(|x| *x == 5));
    }"#;
    assert_eq!(
        unbounded_quantifiers(element_only),
        vec![(4usize, "v".to_owned())],
        "R118: 元素值界不是集合界"
    );
    // 已知红（R119）: 界属于**前缀相近的兄弟根** `bb` ⇒ 根 `b` 仍未受限
    let near_miss = r#"
    fn t() {
        assert_eq!(bb.len(), 8);
        assert!(b.iter().all(|x| *x == 0));
    }"#;
    assert_eq!(
        unbounded_quantifiers(near_miss),
        vec![(4usize, "b".to_owned())],
        "R119: `bb.len()` 不得被当成 `b` 的界"
    );
}

/// 判据 (**配对已知红**): 本检查器认的**每一种界形态**, 都必须有一条"**去掉它就会红**"的配对读数。
///
/// 只有"认"没有"去掉会红" ⇒ 那个形态可能是**惰性**的（R122: 未喂已知红的判据要登记为"可能惰性"）。
/// 本判据把 5 种形态**逐条配对**, 每条都给「**有界 ⇒ 绿** / **去掉界 ⇒ 红**」两个读数。
#[test]
fn every_recognised_bound_form_has_a_paired_known_red() {
    // (形态名, **有界**的片段, **去掉界**的片段)
    let pairs: [(&str, &str, &str); 5] = [
        (
            "①显式 len() >=",
            "\n    fn t() {\n        assert!(v.len() >= 8 && v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "②宏隐式相等 assert_eq!(len, N)",
            "\n    fn t() {\n        assert_eq!(v.len(), 16);\n        assert!(v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "③!is_empty()",
            "\n    fn t() {\n        assert!(!v.is_empty() && v.iter().any(|x| *x == 1));\n    }",
            "\n    fn t() {\n        assert!(v.iter().any(|x| *x == 1));\n    }",
        ),
        (
            "④值界 len() == N",
            "\n    fn t() {\n        assert!(v.len() == 4 && v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "⑥显式断言空表（R125）",
            "\n    fn t() {\n        assert!(v.is_empty(), \"对照\");\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
            "\n    fn t() {\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
        ),
    ];
    assert_eq!(
        pairs.len(),
        REGISTERED_BOUND_FORMS.len(),
        "已注册形态各一对（⛔ 不写死条数）"
    );
    // **R191/R194: 两个配对, 各只差**一个维度**, 方向相反**:
    // 配对一（维度 = **界在不在**）: 同一根上有界 ⇒ 绿; 去掉界 ⇒ 红（上面的 pairs）。
    // 配对二（维度 = **界挂在哪**，界的**存在性固定为"有"**）: 界挂在**被量词的根**上 ⇒ 绿;
    // 把同一个界**搬到邻居根** `w` 上（只差"根的身份"这一个维度）⇒ 必须红。
    let neighbour: [(&str, &str, &str); 5] = [
        (
            "explicit-len-ge",
            "\n    fn t() {\n        assert!(v.len() >= 8 && v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert!(w.len() >= 8 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "macro-implicit-len-eq",
            "\n    fn t() {\n        assert_eq!(v.len(), 16);\n        assert!(v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert_eq!(w.len(), 16);\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "not-is-empty",
            "\n    fn t() {\n        assert!(!v.is_empty() && v.iter().any(|x| *x == 1));\n    }",
            "\n    fn t() {\n        assert!(!w.is_empty() && v.iter().any(|x| *x == 1));\n    }",
        ),
        (
            "value-bound-len-eq",
            "\n    fn t() {\n        assert!(v.len() == 4 && v.iter().all(|x| *x == 0));\n    }",
            "\n    fn t() {\n        assert!(w.len() == 4 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "explicit-empty-table",
            "\n    fn t() {\n        assert!(v.is_empty(), \"对照\");\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
            "\n    fn t() {\n        assert!(w.is_empty(), \"对照\");\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
        ),
    ];
    for (form, on_root, on_neighbour) in neighbour {
        assert!(
            unbounded_quantifiers(on_root).is_empty(),
            "{form}: 界挂在被量词的根上 ⇒ 绿"
        );
        assert!(
            !unbounded_quantifiers(on_neighbour).is_empty(),
            "{form}/R191: 同一个界搬到**邻居根**上（只差根的身份）⇒ 必须红"
        );
    }
    for (form, with_bound, without_bound) in pairs {
        assert!(
            unbounded_quantifiers(with_bound).is_empty(),
            "{form}: 有界必须绿"
        );
        assert!(
            !unbounded_quantifiers(without_bound).is_empty(),
            "{form}: **去掉界必须红**（否则该形态是惰性的 —— R122）"
        );
    }
}

/// 判据: 掩码对**真实源文件**也逐字节等长（R113 的常驻自检）。
#[test]
fn masking_is_byte_length_preserving_for_every_source_file() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut scanned = 0usize;
    for entry in std::fs::read_dir(&root).expect("读 src/") {
        let path = entry.expect("目录项").path();
        if path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let source = std::fs::read_to_string(&path).expect("读源文件");
        let masked = mask(&source);
        assert_eq!(
            masked.len(),
            source.len(),
            "{}: 掩码必须逐字节等长",
            path.display()
        );
        assert_eq!(
            masked.matches('\n').count(),
            source.matches('\n').count(),
            "{}: 换行数必须不变",
            path.display()
        );
        scanned += 1;
    }
    assert!(
        scanned >= 7,
        "至少扫到 7 个源文件（R93: 下界）, 实际 {scanned}"
    );
}

/// **R160 的注册表**: 本检查器**认**的界形态（每条都必须有正对照命中, 且正对照只许命中已注册形态）。
const REGISTERED_BOUND_FORMS: [&str; 5] = [
    "explicit-len-ge",
    "macro-implicit-len-eq",
    "not-is-empty",
    "value-bound-len-eq",
    "explicit-empty-table",
];

/// **R119/R120 的机械下界**: 扫描必须"每个文件都处理过", 且入口数为 0。
/// 返回 `Err` 的理由串, 便于**配对已知红**直接断言它会拒绝坏输入。
fn scan_reached(files: usize, scanned: usize, skipped: usize) -> Result<(), String> {
    if files < 7 {
        return Err(format!(
            "只读到 {files} 个源文件（下界 7）⇒ 扫描面太窄, 结论不可用"
        ));
    }
    if scanned + skipped != files {
        return Err(format!(
            "{scanned} + {skipped} != {files} ⇒ 有文件既没被扫也没被登记"
        ));
    }
    if skipped != 0 {
        return Err(format!("跳过入口 {skipped} 个（要求 0）⇒ 扫描域未封闭"));
    }
    Ok(())
}

/// 判据 (**R214① 的对抗样本**): 掩码必须**既等长又不泄漏**。
///
/// 风险是**对偶的**: 不掩码 ⇒ **假阳性**（注释/字符串里的东西被当成代码）;
/// 掩码**失同步** ⇒ **假阴性**（更危险: 把后续正文当字符串抹掉 ⇒ 掩盖真违规）。
/// 四件套: ①字符串含 `//` ②行注释含 `"` ③行注释含 `//` ④**块**注释含 `"` 与 `//`;
/// 另加三个 Rust 特有陷阱: ⑤**字符字面量**含 `"` ⑥**原始字符串**含 `"` ⑦生命周期 `'a`。
/// 每条样本读**两个数**: 逐字节等长 ＋ **无泄漏**（样本**之后**那条真实违规必须仍被抓到）。
///
/// ⚠ 本判据不是纸面练习: 加它的**同一次**修复, 就在本仓 `als.rs` **露出**一处被
/// 失同步掩码隐藏的真缺口（`losses_matching(..)` 没有根绑定的域）⇒ 已修。
#[test]
fn masking_survives_adversarial_constructs() {
    let cases: [(&str, &str); 7] = [
        (
            "①字符串含 //",
            "fn t() { let s = \"http://x\"; assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "②行注释含 \"",
            "fn t() { // 含 \" 引号\n    assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "③行注释含 //",
            "fn t() { // 含 // 两个斜杠\n    assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "④块注释含 \" 与 //",
            "fn t() { /* \" 与 // */ assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "⑤字符字面量含 \"",
            "fn t() { let q = '\"'; assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "⑥原始字符串含 \"",
            "fn t() { let s = r#\"a \"b\" c\"#; assert!(v.iter().all(|x| *x == 0)); }",
        ),
        (
            "⑦生命周期 `'a`",
            "fn f<'a>(x: &'a str) -> usize { let _ = x; assert!(v.iter().all(|y| *y == 0)); 0 }",
        ),
    ];
    for (name, src) in cases {
        let masked = mask(src);
        assert_eq!(
            masked.len(),
            src.len(),
            "{name}: 掩码必须逐字节等长（R113）"
        );
        assert_eq!(
            masked.matches('\n').count(),
            src.matches('\n').count(),
            "{name}: 换行数必须不变"
        );
        assert!(
            !unbounded_quantifiers(src).is_empty(),
            "{name}: **无泄漏** —— 样本之后的真实违规必须仍被抓到（R214①: 失同步 = 假阴性）"
        );
    }
}

/// 判据 (**配对已知红**: 机械下界): 上界/下界三条件各自**破坏即拒绝**。
#[test]
fn scan_lower_bounds_reject_broken_inputs() {
    assert!(scan_reached(11, 11, 0).is_ok(), "正常输入必须通过");
    // 配对已知红 ①文件太少 ②既没扫也没登记 ③有跳过入口
    assert!(scan_reached(3, 3, 0).is_err(), "少于下界必须拒绝");
    assert!(scan_reached(11, 10, 0).is_err(), "漏扫必须拒绝");
    assert!(scan_reached(11, 10, 1).is_err(), "有跳过入口必须拒绝");
}

/// 判据 (**R160 双向归零**): 形态注册表与正对照表**两个方向都相等**。
#[test]
fn bound_form_registry_is_bidirectionally_zeroed() {
    // 正对照表: 每条已注册形态一个"有界"片段（与 `every_recognised_bound_form_has_a_paired_known_red` 同名）。
    let positive: [(&str, &str); 5] = [
        (
            "explicit-len-ge",
            "\n    fn t() {\n        assert!(v.len() >= 8 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "macro-implicit-len-eq",
            "\n    fn t() {\n        assert_eq!(v.len(), 16);\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "not-is-empty",
            "\n    fn t() {\n        assert!(!v.is_empty() && v.iter().any(|x| *x == 1));\n    }",
        ),
        (
            "value-bound-len-eq",
            "\n    fn t() {\n        assert!(v.len() == 4 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "explicit-empty-table",
            "\n    fn t() {\n        assert!(v.is_empty(), \"对照\");\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
        ),
    ];
    // 方向 ①: 每条**已注册**形态都必须有正对照命中
    for form in REGISTERED_BOUND_FORMS {
        let hit = positive.iter().filter(|(name, _)| *name == form).count();
        assert_eq!(hit, 1, "已注册形态 `{form}` 必须有且只有一个正对照");
    }
    // 方向 ②: 正对照只许命中**已注册**形态（没有"表外形态"被当成界）
    assert_eq!(
        positive.len(),
        REGISTERED_BOUND_FORMS.len(),
        "两个方向的条数必须相等"
    );
    for (name, snippet) in positive {
        assert!(
            REGISTERED_BOUND_FORMS.contains(&name),
            "正对照 `{name}` 不在注册表里 ⇒ 表外形态被当成界（R160 方向 ②）"
        );
        assert!(
            unbounded_quantifiers(snippet).is_empty(),
            "{name}: 正对照必须被认（否则该形态是惰性的）"
        );
    }
    // 方向 ② 的反证: **未注册**形态（运行期计数器, R118）不得被认
    let unregistered = "\n    fn t() {\n        let mut count = 0;\n        for x in v { count += 1; }\n        assert_eq!(count, 4);\n        assert!(v.iter().all(|y| *y == 0));\n    }";
    assert!(
        !unbounded_quantifiers(unregistered).is_empty(),
        "未注册的形态（计数器）不得被当成界"
    );
}

/// 判据 (**注入做成常驻判据**): 三条常驻判据各自的"形态级注入"都在**本判据内**复现,
/// 且每对**绿/红两臂**都要成立 —— 因此批 23 的 3/3 隔离**永久**保住（不再依赖外部驱动器）。
#[test]
fn every_standing_criterion_has_a_synthetic_green_and_red_arm() {
    // 判据 1（扫源文件）: 同一段合成源码, **有界** ⇒ 绿 / **去掉界** ⇒ 红
    let c1_green =
        "\n    fn t() {\n        assert!(v.len() >= 1 && v.iter().all(|x| *x == 0));\n    }";
    let c1_red = "\n    fn t() {\n        assert!(v.iter().all(|x| *x == 0));\n    }";
    assert!(unbounded_quantifiers(c1_green).is_empty(), "判据1 绿臂");
    assert!(
        !unbounded_quantifiers(c1_red).is_empty(),
        "判据1 红臂（去掉界必须被抓）"
    );
    // 判据 2（检查器的牙）: 边界开 ⇒ near-miss 漏判（红臂）/ 边界在 ⇒ 抓住（绿臂）
    let c2_near_miss =
        "\n    fn t() {\n        assert!(ab.len() >= 4 && b.iter().all(|x| *x == 0));\n    }";
    assert!(
        !unbounded_quantifiers(c2_near_miss).is_empty(),
        "判据2 绿臂: 带标识符边界时必须抓住 near-miss"
    );
    assert!(
        !contains_identifier("assert!(ab.len()>=4&&b.iter()", "assert!(b.len()>="),
        "判据2 红臂: 若边界判定失效（`ok = true`）, `b` 会被 `ab.len()` 满足 ⇒ 漏判"
    );
    // 判据 3（掩码逐字节等长）: 正确掩码 ⇒ 等长（绿）/ 写坏的掩码 ⇒ 不等长（红）
    let sample = "// 注释\n    fn t() {\n        let s = \"中文\";\n    }";
    assert_eq!(mask(sample).len(), sample.len(), "判据3 绿臂: 正确掩码等长");
    let broken = sample
        .lines()
        .map(|line| line.split("//").next().unwrap_or(line))
        .collect::<Vec<_>>()
        .join("\n");
    assert_ne!(
        broken.len(),
        sample.len(),
        "判据3 红臂: 写坏的掩码必须不等长"
    );
}

/// **R213**: 去重站点总数的**根绑定**下界 —— 参数就是**真正被搜的那个集合**。
/// 返回 `Err` 的理由, 便于**行为臂**直接断言"缩到界之下会红"。
fn assert_site_floor(sources: &[String]) -> Result<usize, String> {
    let total: usize = sources
        .iter()
        .map(|src| unbounded_quantifiers_with(&[], src).len())
        .sum();
    if sources.len() < 7 {
        return Err(format!("被搜集合只剩 {} 个文件（下界 7）", sources.len()));
    }
    if total < 20 {
        return Err(format!("去重站点总数 {total} 低于下界 20"));
    }
    Ok(total)
}

/// 判据 (**R227②/③ ＋ R215①**): 计数类守卫的**两个盲区各一例**, 且**双向常驻**
/// （每一步都同时断言「**旧守卫判真**」＋「**新谓词抓到缺陷**」）。
#[test]
fn count_floors_cannot_see_defects_in_either_direction() {
    let count = |src: &str| unbounded_quantifiers_with(&[], src).len();
    let defect = |src: &str| !unbounded_quantifiers(src).is_empty();
    // 盲区 ① **计数不变**: 同一段源码, 只把**界**去掉 ⇒ 计数一字不变, 而缺陷出现（R225③ 的常驻形态）
    let with_bound =
        "\n    fn t() {\n        assert!(v.len() >= 1 && v.iter().all(|x| *x == 0));\n    }";
    let bound_removed = "\n    fn t() {\n        assert!(v.iter().all(|x| *x == 0));\n    }";
    assert_eq!(
        count(with_bound),
        count(bound_removed),
        "R227②①: 去掉界**不改变计数** ⇒ 计数类守卫看不见它"
    );
    assert!(!defect(with_bound), "有界 ⇒ 新谓词判无缺陷");
    assert!(defect(bound_removed), "去掉界 ⇒ 新谓词必须抓到");
    // 盲区 ② **计数大幅上升**: 大量**良构**站点 ＋ 1 个缺陷（`engine` 的 127 → 358 同构）
    let mut inflated = String::new();
    for i in 0..120 {
        inflated.push_str(&format!(
            "\n    fn f{i}() {{\n        assert!(v.len() >= 1 && v.iter().all(|x| *x == {i}));\n    }}"
        ));
    }
    let honest = count(&inflated);
    inflated.push_str("\n    fn bad() {\n        assert!(w.iter().all(|y| *y == 0));\n    }");
    let polluted = count(&inflated);
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R227-INFLATE] 站点计数 {honest} → {polluted}（旧地板 >= 20 照样通过）"
    );
    assert!(polluted > honest, "② 计数必须**上升**");
    assert!(polluted >= 20, "② 旧守卫（计数地板）**照样通过**");
    assert!(defect(&inflated), "② 新谓词必须抓到那个缺陷");
    // **R215①**: 逐地板**余量**（实测 − 下界）与**最小余量**, 全部**当场从源码算**（⛔ 不写死读数, R226②）
    let floors: [(&str, usize); 5] = [
        ("explicit-len-ge", 5),
        ("macro-implicit-len-eq", 5),
        ("not-is-empty", 10),
        ("value-bound-len-eq", 1),
        ("explicit-empty-table", 0),
    ];
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let sources: Vec<String> = std::fs::read_dir(root.join("src"))
        .expect("读 src/")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| std::fs::read_to_string(path).expect("读源文件"))
        .collect();
    let mut min_margin = usize::MAX;
    let mut min_form = "";
    for (form, floor) in floors {
        let measured: usize = sources
            .iter()
            .map(|src| {
                total_per_file_offenders(src) - unbounded_quantifiers_with(&[form], src).len()
            })
            .sum();
        // **R235**: "显式断言空表"在本仓实测 0 ⇒ 它是 **golden-pin 类**（由**合成正对照**行使）
        // ⇒ 余量**无定义**, ⛔ 不是 0。
        if form == "explicit-empty-table" {
            eprintln!(
                "[R187-PROBE render/tests/quantifier_lower_bounds R215-MARGINS] 形态 `{form}`: 实测 {measured}（golden-pin 类, 由合成正对照行使）⇒ **余量无定义**（⛔ 不是 0）"
            );
            continue;
        }
        let margin = measured - floor;
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R215-MARGINS] 形态 `{form}`: 实测 {measured} − 下界 {floor} = 余量 **{margin}**"
        );
        // **R236②**: 余量为 0 时必须**显式告警**（"非零"看不出"再删一条就破"）
        if margin == 0 {
            eprintln!(
                "[R187-PROBE render/tests/quantifier_lower_bounds R236-WARN] ⚠ 形态 `{form}` 余量为 **0** ⇒ 无缓冲: 实测值再降 1 就会红。**原因**: 本仓只有 1 处该形态的界; 删掉那处界会让判据**按设计**变红（⛔ 这不是回归, 而是判据在履职）"
            );
        }
        if margin < min_margin {
            min_margin = margin;
            min_form = form;
        }
    }
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R215-MARGINS] 最小余量 = {min_margin}（形态 `{min_form}`）"
    );
    assert!(min_margin < usize::MAX, "余量表必须非空");
}

/// **R243② 的辅助**: 一行里出现的**原始字符串开口**（四形态分开: `r"` / `r#"` / `br"` / `br#"`）。
/// 带**标识符前缀边界**（前一个字节不是字母/数字/下划线）—— 否则 `for#"` 之类会被误计。
fn raw_openings_in_line(line: &str) -> Vec<&'static str> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let prev_ok = i == 0 || !(bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
        if prev_ok {
            let rest = &line[i..];
            for kind in ["br#\"", "br\"", "r#\"", "r\""] {
                if rest.starts_with(kind) {
                    out.push(kind);
                    i += kind.len();
                    break;
                }
            }
        }
        // ⚠ 必须按**字符**前进: 按字节 +1 会落在 UTF-8 中间 ⇒ `&line[i..]` panic（本机实测）。
        i += line[i..].chars().next().map_or(1, |ch| ch.len_utf8());
    }
    out
}

/// 判据 (**R237② ＋ R243②: 语料预检先于臂设计; 读数是**逐线本地**的**):
/// ① 逐**形态**分开报（原始字符串 `r"` / `r#"` / `br"` / `br#"` **四个都单列**）;
/// ② 声明**偏置**（计数是**上界**: 可能命中字符串/注释里的同形文本; 语料**不含** `tests/` 里的臂夹具）;
/// ③ **标明逐线本地**（⛔ 跨线搬数会出错 —— 本线自己的数才算数）;
/// ④ 给出**命中行样例**（R184: 清单要**读**, ⛔ 不只数个数）。
#[test]
fn corpus_precheck_reports_the_trigger_surface_of_every_construct() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<(String, String)> = std::fs::read_dir(root.join("src"))
        .expect("读 src/")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| {
            (
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                std::fs::read_to_string(&path).expect("读源文件"),
            )
        })
        .collect();
    files.sort();
    assert!(!files.is_empty(), "语料必须非空");
    // 逐构造: (显示名, 判定函数) —— 逐一给 **计数 ＋ 前 2 条命中行样例**
    let raw_shapes = ["br#\"", "br\"", "r#\"", "r\""];
    for shape in raw_shapes {
        let mut count = 0usize;
        let mut samples: Vec<String> = Vec::new();
        for (name, text) in &files {
            for (index, line) in text.lines().enumerate() {
                if raw_openings_in_line(line).contains(&shape) {
                    count += 1;
                    if samples.len() < 2 {
                        samples.push(format!("{name}:{}", index + 1));
                    }
                }
            }
        }
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R243-CORPUS] 原始字符串形态 `{shape}`: **{count}** 次; 样例 {samples:?}"
        );
    }
    let line_constructs: [(&str, &str); 4] = [
        ("字符字面量含双引号", "'\"'"),
        ("生命周期 `'a`", "'a"),
        ("块注释 `/*`", "/*"),
        ("行注释 `//`", "//"),
    ];
    for (label, needle) in line_constructs {
        let mut count = 0usize;
        let mut samples: Vec<String> = Vec::new();
        for (name, text) in &files {
            for (index, line) in text.lines().enumerate() {
                if line.contains(needle) {
                    count += 1;
                    if samples.len() < 2 {
                        samples.push(format!("{name}:{}", index + 1));
                    }
                }
            }
        }
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R243-CORPUS] 构造 `{label}`: **{count}** 次; 样例 {samples:?}"
        );
    }
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R243-CORPUS] 偏置声明: 上述计数是**上界**（可命中字符串/注释里的同形文本）; 语料 = `src/*.rs`（⛔ 不含 `tests/` 的臂夹具）; **逐线本地**: ⛔ 不得跨线搬用"
    );
    // **R237 补充（风险声明: 类别 ＋ 触发面）**
    let trigger = files
        .iter()
        .map(|(_, text)| text.matches("'\"'").count())
        .sum::<usize>();
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R237-RISK] 掩码风险类别 = 假阴性（失同步 ⇒ 抹掉后续正文）; 触发面 = `'\"'` **{trigger}** 次（本线本地, 当场算）"
    );
}

/// 把两段文本按**字节**对齐成两行, 空格显示为 `·`（掩码可见）—— **R237① 的裁判**。
fn two_row_referee(fixture: &str, masked: &str, upto: usize) -> (String, String) {
    let a: Vec<u8> = fixture.bytes().take(upto).collect();
    let b: Vec<u8> = masked.bytes().take(upto).collect();
    let render = |row: &[u8]| {
        row.iter()
            .map(|byte| match byte {
                b' ' => '·',
                b'\n' => '⏎',
                _ => *byte as char,
            })
            .collect::<String>()
    };
    (render(&a), render(&b))
}

/// 判据 (**R239①/R241①/R242③: 判缺陷前先判**样本合法性**; 夹具必须**自证其形状****):
/// 每条臂的样本先声明它**必须含**什么、**必须不含**什么, 并**当场断言**。
/// ⇒ 臂变红时不再"默默指控自己"（批 28 的样本 ⑤ 正是栽在这里: 红的是**实现**, 却被当成夹具问题）。
#[test]
fn every_arm_sample_proves_its_own_shape() {
    let root = "v";
    let checks: [(&str, &str, &[&str], &[&str]); 5] = [
        (
            "explicit-len-ge",
            "assert!({r}.len() >= 8 && {r}.iter().all(|x| *x == 0));",
            // ⚠ 自证断言在**替换之后**检查 ⇒ 必须写替换后的形状（本机实测: 写 `{r}` 会被自己抓住）
            &[".len() >=", ".all(", "v.iter()"],
            &["w.len()"],
        ),
        (
            "macro-implicit-len-eq",
            "assert_eq!({r}.len(), 16);",
            &["assert_eq!(", ".len(),"],
            &[".all("],
        ),
        (
            "not-is-empty",
            "assert!(!{r}.is_empty() && {r}.iter().any(|x| *x == 1));",
            // 自证断言在**替换之后**检查 ⇒ 写替换后的形状
            &["!v.is_empty()", ".any("],
            &["w.is_empty()"],
        ),
        (
            "value-bound-len-eq",
            "assert!({r}.len() == 4 && {r}.iter().all(|x| *x == 0));",
            &[".len() ==", ".all("],
            &["w.len()"],
        ),
        (
            "explicit-empty-table",
            "assert!({r}.is_empty(), \"对照\");",
            &["v.is_empty()"],
            &["!v.is_empty()"],
        ),
    ];
    assert_eq!(
        checks.len(),
        REGISTERED_BOUND_FORMS.len(),
        "每个已注册形态都要有自证样本"
    );
    for (form, template, required, forbidden) in checks {
        let sample = template.replace("{r}", root);
        for needle in required {
            assert!(
                sample.contains(needle),
                "R239①: 形态 `{form}` 的样本必须**自证**含 `{needle}`（样本: {sample}）"
            );
        }
        for needle in forbidden {
            assert!(
                !sample.contains(needle),
                "R239①: 形态 `{form}` 的样本必须**自证不含** `{needle}`（样本: {sample}）"
            );
        }
        // 量化器计数: 断言样本里的 `.all(`/`.any(` 数量（⛔ 防止"零量化器"的假样本）
        let quantifiers = sample.matches(".all(").count() + sample.matches(".any(").count();
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R239-SHAPE] 形态 `{form}`: 必含 {required:?} 必不含 {forbidden:?} 量化器数 = {quantifiers}"
        );
        assert!(quantifiers <= 1, "自证样本不该含多个量化器: {sample}");
    }
    // 掩码对抗样本的自证（R239①: 先判样本合法性）
    let mask_samples: [(&str, &str); 3] = [
        ("含 `'\"'`", "let q = '\"';"),
        ("含 `r#\"`", "let s = r#\"x\"#;"),
        ("含块注释", "/* x */"),
    ];
    for (label, sample) in mask_samples {
        let construct_ok = match label {
            "含 `'\"'`" => sample.contains("'\"'"),
            "含 `r#\"`" => sample.contains("r#\""),
            _ => sample.contains("/*") && sample.contains("*/"),
        };
        assert!(
            construct_ok,
            "R239①: 掩码样本必须自证其形状: {label} / {sample}"
        );
        let masked = mask(sample);
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R239-SHAPE] 掩码样本 `{label}`: 逐字节等长 = {}",
            masked.len() == sample.len()
        );
        assert_eq!(masked.len(), sample.len(), "自证: 掩码必须等长");
    }
}

/// 判据 (**R233④/R234③: 同一判据内的臂 ⇒ 首败即停 ⇒ 日志只覆盖 1/N**):
/// 逐判据给出**臂数 N**（当场从结构算出）＋ 明确写出"若首条臂失败, 单轮取证只覆盖 1/N"。
#[test]
fn arm_inventory_reports_first_failure_coverage() {
    let samples = 7usize; // `masking_survives_adversarial_constructs` 的样本数
    let pairs = REGISTERED_BOUND_FORMS.len();
    let inventory: [(&str, usize); 6] = [
        ("masking_survives_adversarial_constructs", samples * 3),
        (
            "every_recognised_bound_form_has_a_paired_known_red",
            pairs * 4,
        ),
        (
            "each_registered_form_is_individually_witnessed",
            pairs * pairs + pairs,
        ),
        ("each_registered_form_has_its_own_lower_bound", pairs + 1),
        (
            "count_floors_cannot_see_defects_in_either_direction",
            3 + pairs,
        ),
        ("char_literal_fixture_is_refereed_byte_by_byte", 6),
    ];
    let mut total = 0usize;
    for (index, (criterion, arms)) in inventory.iter().enumerate() {
        total += *arms;
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R233-ARMS] 第 **{}/{n}** 条判据 `{criterion}`: 臂数 **{arms}**（首败即停 ⇒ 单轮取证最多覆盖 1/{arms}）",
            index + 1,
            n = inventory.len()
        );
        assert!(*arms >= 1, "`{criterion}` 必须至少有一个臂");
    }
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R233-ARMS] 清单内臂数合计 = **{total}**（⛔ 不可与站点数/判据数相加, R208）"
    );
    assert!(total >= inventory.len(), "臂数合计不得小于判据数");
}

/// 判据 (**R237①: 逐字节打印是"模型 vs 实现"的唯一裁判**):
/// 对 `'"'`（**字符字面量含引号**）这一个夹具, 一次打印就能把"**夹具写错**"与"**实现错**"分开。
/// 三条正特征必须**同时**成立, 否则臂无牙:
/// ① 那三个字节（`'` `"` `'`）被掩掉; ② 其后的 `;` **仍可见**（未被吞掉）;
/// ③ 掩码后仍**含**后半段的界文本（`len() >= 3`）。
/// ⚠ **R210**: 本判据只做裁判, ⛔ 不把它当成"抓到了什么"。
#[test]
fn char_literal_fixture_is_refereed_byte_by_byte() {
    let fixture = "fn t() { let q = '\"'; assert!(v.len() >= 3 && v.iter().all(|x| *x == 0)); }";
    let masked = mask(fixture);
    let (row_fixture, row_masked) = two_row_referee(fixture, &masked, 40);
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R237-REFEREE] 夹具 = {row_fixture}"
    );
    eprintln!("[R187-PROBE render/tests/quantifier_lower_bounds R237-REFEREE] 掩码 = {row_masked}");
    // ① 三字节被掩
    let quote_at = fixture
        .find("'\"'")
        .expect("夹具必须含 单引号+双引号+单引号");
    assert_eq!(
        &masked[quote_at..quote_at + 3],
        "   ",
        "① `'\"'` 的三个字节必须被掩成空格（裁判行: {row_masked}）"
    );
    // ② 其后的 `;` 仍可见
    let semi_at = quote_at + 3;
    assert_eq!(
        fixture.as_bytes()[semi_at],
        b';',
        "夹具必须紧跟一个 `;`（模型如此）"
    );
    assert_eq!(
        masked.as_bytes()[semi_at],
        b';',
        "② 其后的 `;` 必须**仍可见** ⇒ ⛔ 实现不许把后续正文一起吞掉（R214① 的假阴性）"
    );
    // ③ 掩码后仍含后半段的界文本
    assert!(
        masked.contains("len() >= 3"),
        "③ 掩码后必须**仍含**后半段的界文本 ⇒ 后面的结构没有被当作字符串抹掉"
    );
    // 交叉: 该夹具的两个极性都要成立（有界 ⇒ 绿; 去掉界 ⇒ 红）
    assert!(
        unbounded_quantifiers(fixture).is_empty(),
        "正极（有界）必须绿"
    );
    let no_bound = fixture.replace("v.len() >= 3 && ", "");
    assert!(
        !unbounded_quantifiers(&no_bound).is_empty(),
        "负极（去掉界）必须红"
    );
}

/// 判据 (**R227①: 样本必须同时命中判别器的每一个正特征, 否则臂无牙**):
/// 逐形态列出**它的正对照命中了哪些针**（`eprintln!` 可检索), 并断言:
/// ① 该形态在自己的**每个路径**（condition/body）上至少命中一根针;
/// ② 该样本**不得**命中**其它**形态的针（否则"只有它会红"就不是该形态的功劳）。
#[test]
fn every_form_control_hits_all_required_needles() {
    let root = "v";
    let controls: [(&str, &str); 5] = [
        (
            "explicit-len-ge",
            "assert!({r}.len() >= 8 && {r}.iter().all(|x| *x == 0));",
        ),
        (
            "macro-implicit-len-eq",
            "assert_eq!({r}.len(), 16);\n    assert!({r}.iter().all(|x| *x == 0));",
        ),
        (
            "not-is-empty",
            "assert!(!{r}.is_empty() && {r}.iter().any(|x| *x == 1));",
        ),
        (
            "value-bound-len-eq",
            "assert!({r}.len() == 4 && {r}.iter().all(|x| *x == 0));",
        ),
        (
            "explicit-empty-table",
            "assert!({r}.is_empty(), \"对照\");\n    assert!(!{r}.iter().any(|x| *x == 1));",
        ),
    ];
    assert_eq!(
        controls.len(),
        REGISTERED_BOUND_FORMS.len(),
        "每个已注册形态各一个正对照"
    );
    for (form, template) in controls {
        let body = template.replace("{r}", root);
        let cond = strip_ws(&body);
        // ① 逐针判定: 该形态的每根针**是否命中**
        let mut hit_paths: Vec<&str> = Vec::new();
        let mut required_paths: Vec<&str> = Vec::new();
        for (path, needle) in form_needles(form, root) {
            if !required_paths.contains(&path) {
                required_paths.push(path);
            }
            // 正对照的文本已经把 condition 与 body 两段都写进来了 ⇒ 两条路径都在这段文本上判针。
            let hay = &cond;
            if contains_identifier(hay, &strip_ws(&needle)) {
                hit_paths.push(path);
            }
        }
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R227-NEEDLES] 形态 `{form}`: 路径 {required_paths:?} 命中 {hit_paths:?}"
        );
        for path in &required_paths {
            assert!(
                hit_paths.contains(path),
                "R227①: 形态 `{form}` 的样本没有命中 `{path}` 路径的任何正特征 ⇒ 臂无牙"
            );
        }
        // ② 交叉: 该样本不得命中**其它**形态的针（判别的功劳归属该形态）
        for other in REGISTERED_BOUND_FORMS {
            if other == form {
                continue;
            }
            let cross = form_needles(other, root)
                .iter()
                .any(|(_, needle)| contains_identifier(&cond, &strip_ws(needle)));
            assert!(
                !cross,
                "R227①: 形态 `{form}` 的样本**命中**了 `{other}` 的针 ⇒ 隔离性不成立"
            );
        }
    }
}

/// 判据 (**R213 的行为臂**): 把**被搜集合**缩到界之下 ⇒ 必须红; 正常集合 ⇒ 绿。
#[test]
fn site_floor_reddens_when_the_searched_set_collapses() {
    // 绿臂: 7 个文件（≥ 7）且每个 3 个站点（合计 21 ≥ 20）⇒ 两个条件都满足
    let real: Vec<String> = ["a", "b", "c", "d", "e", "f", "g"]
        .iter()
        .map(|_| {
            "fn t() { assert!(v.iter().all(|x| *x == 0)); assert!(w.iter().all(|y| *y == 1)); assert!(u.iter().any(|z| *z == 2)); }"
                .to_owned()
        })
        .collect();
    // R226②: 期望值**当场计算**（⛔ 不写死数字）
    let expected_sites = real.len() * 3;
    assert_eq!(
        assert_site_floor(&real).expect("正常集合必须通过"),
        expected_sites,
        "{} 个文件 × 3 个站点",
        real.len()
    );
    // 行为臂: 集合塌缩 ⇒ 必须 `Err`（并且是**集合太小**这个理由）
    let collapsed: Vec<String> = vec!["fn t() {}".to_owned(); 2];
    let err = assert_site_floor(&collapsed).expect_err("缩到 2 个文件必须红");
    assert!(
        err.contains("被搜集合"),
        "红理由必须点名**被搜集合**: {err}"
    );
    // 行为臂 2: 文件数够但站点太少 ⇒ 也必须红
    let few_sites: Vec<String> = vec!["fn t() {}".to_owned(); 8];
    assert!(
        assert_site_floor(&few_sites).is_err(),
        "站点总数不足也必须红（R213）"
    );
}

/// 判据 (**逐形态隔离见证**, R182): 对 5 条已注册形态**逐个**做"停用它"的见证 ——
/// 停用形态 `f` 后, **只有 `f` 的正对照**变红, 其余四个仍绿 ⇒ 每个形态都有**独有**的牙。
#[test]
fn each_registered_form_is_individually_witnessed() {
    let controls: [(&str, &str); 5] = [
        (
            "explicit-len-ge",
            "\n    fn t() {\n        assert!(v.len() >= 8 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "macro-implicit-len-eq",
            "\n    fn t() {\n        assert_eq!(v.len(), 16);\n        assert!(v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "not-is-empty",
            "\n    fn t() {\n        assert!(!v.is_empty() && v.iter().any(|x| *x == 1));\n    }",
        ),
        (
            "value-bound-len-eq",
            "\n    fn t() {\n        assert!(v.len() == 4 && v.iter().all(|x| *x == 0));\n    }",
        ),
        (
            "explicit-empty-table",
            "\n    fn t() {\n        assert!(v.is_empty(), \"对照\");\n        assert!(!v.iter().any(|x| *x == 1));\n    }",
        ),
    ];
    // 全开 ⇒ 五条正对照都必须绿（先证基线）
    for (form, snippet) in controls {
        assert!(
            unbounded_quantifiers(snippet).is_empty(),
            "{form}: 全开时必须绿"
        );
    }
    for disabled in REGISTERED_BOUND_FORMS {
        let enabled: Vec<&str> = REGISTERED_BOUND_FORMS
            .iter()
            .copied()
            .filter(|f| *f != disabled)
            .collect();
        let mut red = Vec::new();
        for (form, snippet) in controls {
            if !unbounded_quantifiers_with(&enabled, snippet).is_empty() {
                red.push(form);
            }
        }
        assert_eq!(
            red,
            vec![disabled],
            "停用 `{disabled}` 后必须**只有它**的正对照变红（R182: 逐形态独有的牙）"
        );
    }
}

/// 判据 (**R185**: 每个形态都要有**自己的下界**): 逐形态给出两条读数 ——
/// ① **合成正对照数**（必须恰为 1, 这是该形态的机械下界）;
/// ② **本仓实际命中数**（实测并打印; ⛔ 不许"某形态 0 条"静默合法）。
#[test]
fn each_registered_form_has_its_own_lower_bound() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("src"))
        .expect("读 src/")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();
    let sources: Vec<String> = files
        .iter()
        .map(|path| std::fs::read_to_string(path).expect("读源文件"))
        .collect();
    // **R204 的两种量法（定义必须写明）**:
    // ① **站点去重总数** = `enabled = &[]` 时每个站点都算"无界" ⇒ 逐文件求和（本变量 `total`）;
    // ② **形态归属之和** = 只开单个形态时被它认下的站点数之和（可重叠 ⇒ ⛔ 比 ① 大）。
    let total: usize = sources
        .iter()
        .map(|src| unbounded_quantifiers_with(&[], src).len())
        .sum();
    // **R203**: 探针读数写成 `eprintln!`（⛔ 不写进断言消息 —— 通过的断言在**任何模式**下都不打印）。
    eprintln!(
        "[R187-PROBE render/tests/quantifier_lower_bounds R204-SITES] 站点去重总数 total = {total}（定义 ①）"
    );
    // **R213 的裁定（与 R199 分属两类, ⛔ 不要删）**:
    // - **R199 是"扫描量"地板**: 缺陷会**抬高**计数 ⇒ 计数类地板挡不住它;
    // - **本界守"被搜集合塌缩"**: 集合太小 ⇒ 结论不可用 ⇒ 必须**保留**, 并**根绑定到被搜集合**
    //   （`sources` 就是真正读进来的那批文件）＋ 配**行为臂**（把集合缩到界之下 ⇒ 必须变红）。
    // 上面那条 `eprintln!` 读数继续保留（R204 的可检索量法）。
    assert_site_floor(&sources).expect("被搜集合不得塌缩（R213）");
    // **R185 的逐形态下界（地板全部从**实测**来, R134; 实测值 14/15/23/1/0 ⇒ 留余量）**:
    // 前四种形态在本仓有真实命中; 第五种（"显式断言空表"）在本仓**实测为 0** ——
    // ⛔ 不许让"0 条"静默合法 ⇒ 它的下界是**合成正对照**（恰 1 条）, 且必须由
    // `each_registered_form_is_individually_witnessed` 提供**独有**的牙。这一行把 0 变成**登记**。
    let floors: [(&str, usize); 5] = [
        ("explicit-len-ge", 5),
        ("macro-implicit-len-eq", 5),
        ("not-is-empty", 10),
        ("value-bound-len-eq", 1),
        ("explicit-empty-table", 0),
    ];
    assert_eq!(
        floors.len(),
        REGISTERED_BOUND_FORMS.len(),
        "R185: 每个已注册形态都必须有自己的下界（双向）"
    );
    for (form, floor) in floors {
        assert!(
            REGISTERED_BOUND_FORMS.contains(&form),
            "下界表里的 `{form}` 不在注册表里"
        );
        let alone: usize = sources
            .iter()
            .map(|src| {
                total_per_file_offenders(src) - unbounded_quantifiers_with(&[form], src).len()
            })
            .sum();
        eprintln!(
            "[R187-PROBE render/tests/quantifier_lower_bounds R185-FORMS] 形态 `{form}`: 本仓命中 {alone} 个站点（下界 {floor}）"
        );
        assert!(
            alone >= floor,
            "形态 `{form}` 的本仓命中 {alone} 低于下界 {floor}"
        );
        if floor == 0 {
            assert_eq!(
                alone, 0,
                "0 下界的形态必须**实测 0**（它的界由合成正对照 ＋ 隔离见证承担）"
            );
        }
    }
}

/// 单文件的站点总数（`enabled = &[]`）。
fn total_per_file_offenders(source: &str) -> usize {
    unbounded_quantifiers_with(&[], source).len()
}
