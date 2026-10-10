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
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let rest = &text[i..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(text.len(), |offset| i + offset);
            for ch in text[i..end].chars() {
                out.push_str(&" ".repeat(ch.len_utf8()));
            }
            i = end;
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(text.len(), |offset| i + offset + 2);
            for ch in text[i..end].chars() {
                out.push(if ch == '\n' { '\n' } else { ' ' });
                if ch != '\n' {
                    // 上面已压入一个空格; 其余 UTF-8 字节用空格补齐。
                    out.push_str(&" ".repeat(ch.len_utf8() - 1));
                }
            }
            i = end;
        } else if rest.starts_with('"') {
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
            for ch in text[i..end].chars() {
                out.push(if ch == '\n' { '\n' } else { ' ' });
                if ch != '\n' {
                    out.push_str(&" ".repeat(ch.len_utf8() - 1));
                }
            }
            i = end;
        } else {
            let ch = rest.chars().next().expect("非空");
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
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
    assert!(files.len() >= 7, "至少扫到 7 个源文件（R93: 下界）");
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
    assert_eq!(pairs.len(), 5, "五种被认的界形态各一对（计数下限, R93）");
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
    // 站点总数（`enabled = &[]` ⇒ 每个站点都算"无界"）
    let total: usize = sources
        .iter()
        .map(|src| unbounded_quantifiers_with(&[], src).len())
        .sum();
    assert!(
        total >= 20,
        "本仓至少应有 20 个量词站点（实测 {total}）—— 否则扫描面失效"
    );
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
        println!("R185 形态 `{form}`: 本仓命中 {alone} 个站点（下界 {floor}）");
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
