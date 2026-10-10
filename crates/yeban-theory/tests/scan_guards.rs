//! 常驻扫描器判据（裁决 R115：**一次性审计 ⛔ 不等于判据**）。
//!
//! 本文件把第十/十一批的**一次性审计**升级为**常驻判据**：
//!
//! 1. [`scan_loops_are_bound_and_the_classifier_has_positive_and_negative_controls`]
//!    —— "界的五种形态"（R111）＋ **按本断言的实参解析**（R114，⛔ 不用固定字符窗口）＋
//!    **掩码逐字节等长**（R113）＋ **每种形态一条已知绿 ＋ 一条"无界"已知红**（R56/R112）。
//! 2. [`no_criterion_reads_a_runtime_external_resource`]
//!    —— R100/R116：判据**不得**读运行期外部资源（外部夹具缺失是一种真空形态）。
//!
//! ⚠ 与 `src/lib.rs::crate_has_no_hidden_nondeterminism_sources` 的分工：
//! 那条守卫只看**生产代码到 `#[cfg(test)]` 为止**，测试代码**故意豁免**
//! （理由：测试里允许 `&`、`Vec` 等辅助工具）。⇒ 本文件的第 2 条判据
//! 填的是"**判据自身**不读外部资源"这个缺口。

use yeban_theory::genre::{GenreLibrary, SOURCE_TRADITIONAL_THEORY};
use yeban_theory::melody::{MelodyConstraints, melody_over_chords};
use yeban_theory::pitch::PitchClass;
use yeban_theory::progression::{Meter, expand_progression};
use yeban_theory::rhythm::metric_grid;
use yeban_theory::scale::{Scale, ScaleKind};

// ---------------------------------------------------------------------------
// R113：掩码必须**逐字节等长**
// ---------------------------------------------------------------------------

/// 把字符串字面量、行注释、块注释**逐字节**替换成空格（保留 `\n`）。
///
/// ⭐ **逐字节等长**（R113）：被掩码的字节各替换成**一个**空格字节，
/// 因此 `masked.len() == raw.len()` **恒成立**（函数自己断言这一点）。
/// 若按"每个字符一个空格"补，多字节 UTF-8 会**缩短**字节长度，
/// 凡用偏移在掩码/原文之间映射的地方都会**静默跳过**。
fn mask_preserving_len(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = raw.to_owned().into_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'"' => {
                let start = index;
                index += 1;
                while index < bytes.len() {
                    if bytes[index] == b'\\' {
                        index += 2;
                        continue;
                    }
                    if bytes[index] == b'"' {
                        index += 1;
                        break;
                    }
                    index += 1;
                }
                for byte in out.iter_mut().take(index.min(bytes.len())).skip(start) {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'/' => {
                let start = index;
                while index < bytes.len() && bytes[index] != b'\n' {
                    index += 1;
                }
                for byte in out.iter_mut().take(index).skip(start) {
                    *byte = b' ';
                }
            }
            b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
                let start = index;
                index += 2;
                while index + 1 < bytes.len() && !(bytes[index] == b'*' && bytes[index + 1] == b'/')
                {
                    index += 1;
                }
                index = (index + 2).min(bytes.len());
                for byte in out.iter_mut().take(index).skip(start) {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
            }
            _ => index += 1,
        }
    }
    let masked = String::from_utf8(out).expect("masking only writes ASCII spaces");
    // ⭐ R113 自检：逐字节等长。
    assert_eq!(
        masked.len(),
        raw.len(),
        "R113: the mask must preserve byte length"
    );
    masked
}

// ---------------------------------------------------------------------------
// R114：按**本断言的实参**解析，⛔ 不用固定字符窗口
// ---------------------------------------------------------------------------

/// 从 `masked[start..]` 处（`(` 之后）取出**顶层实参**文本（括号深度感知）。
fn top_level_args(masked: &str, open_paren: usize) -> Vec<String> {
    let bytes = masked.as_bytes();
    let mut depth = 1i32;
    let mut index = open_paren + 1;
    let mut start = index;
    let mut args = Vec::new();
    while index < bytes.len() && depth > 0 {
        match bytes[index] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    args.push(masked[start..index].to_owned());
                    break;
                }
            }
            b',' if depth == 1 => {
                args.push(masked[start..index].to_owned());
                start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    args
}

/// 归一化空白。
fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------------------
// 界的形态（R111）
// ---------------------------------------------------------------------------

/// 六种形态的位标志。
const FORM_EXPLICIT_LEN: u8 = 1 << 0; // ① assert!(x.len() >= N)
const FORM_MACRO_LEN: u8 = 1 << 1; // ② assert_eq!(x.len(), N)  ← 宏隐式相等
const FORM_NOT_EMPTY: u8 = 1 << 2; // ③ assert!(!x.is_empty())
const FORM_VALUE: u8 = 1 << 3; // ④ assert_eq!(累加器, 常量)
const FORM_COUNTER: u8 = 1 << 4; // ⑤ 运行期计数器
const FORM_SUBSET: u8 = 1 << 5; // ⑥ 接收者作用域的**子集计数**（⛔ 不界定集合）

/// ⭐ **R160 注册表**：六种形态**全部**登记在这里。
///
/// ⛔ 只有这四个**界定集合大小**（进入判定）：① ② ③ ⑤。
/// ④（聚合值界）与 ⑥（子集计数）**已登记但不定界** —— 登记它们的目的是让
/// `classify_assertion` 能**报出**它们，从而使 R160 的"双向归零"可检查。
const FORM_REGISTRY: [(u8, &str); 6] = [
    (FORM_EXPLICIT_LEN, "① explicit len >=/>/== N"),
    (FORM_MACRO_LEN, "② macro-implicit assert_eq!(len, N)"),
    (FORM_NOT_EMPTY, "③ !is_empty()"),
    (
        FORM_VALUE,
        "④ aggregate/value bound (does NOT bound the set)",
    ),
    (
        FORM_COUNTER,
        "⑤ runtime counter (only when incremented once per iteration)",
    ),
    (
        FORM_SUBSET,
        "⑥ receiver-scoped subset count (does NOT bound the traversed set)",
    ),
];

/// 界定**集合大小**的形态集合（R118）。
const FORM_SET_SIZE: u8 = FORM_EXPLICIT_LEN | FORM_MACRO_LEN | FORM_NOT_EMPTY | FORM_COUNTER;

/// 收集 `body` 里声明的 `let mut <ident>`（形态 ④/⑤ 的载体）。
fn mutable_locals(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(position) = rest.find("let mut ") {
        let after = &rest[position + "let mut ".len()..];
        let ident: String = after
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if ident.is_empty() {
            break;
        }
        let advance = ident.len();
        out.push(ident);
        rest = &after[advance.min(after.len())..];
        if rest.is_empty() {
            break;
        }
        rest = &rest[1.min(rest.len())..];
    }
    out
}

/// 循环体内被**改动**的累加器标识（`x += ..` / `x.push(..)` / `x.extend(..)` …）。
///
/// ⭐ R114 的"根绑定"在这里体现为：形态 ④/⑤ **必须**由"在本循环体内被改动"的标识承担，
/// 因此不会"借用邻居的 `>=`"。
fn mutated_idents(body: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for method in ["+=", ".push(", ".extend(", ".insert(", ".add("] {
        let mut rest = body;
        while let Some(position) = rest.find(method) {
            let head = rest[..position].trim_end();
            let ident: String = head
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            if !ident.is_empty() && !out.contains(&ident) {
                out.push(ident);
            }
            rest = &rest[position + method.len()..];
        }
    }
    out
}

/// 该标识是否在本循环体内**无条件**自增 1（`x += 1` 或 `x = x + 1`，且在花括号深度 0 处）。
///
/// ⭐ R118：条件自增（写在 `if`/内层 `for` 里）不构成集合大小的界。
fn unconditional_increment(body: &str, ident: &str) -> bool {
    let patterns = [format!("{ident} += 1"), format!("{ident} = {ident} + 1")];
    let bytes = body.as_bytes();
    for pattern in patterns {
        let mut rest = body;
        while let Some(position) = rest.find(&pattern) {
            let absolute = body.len() - rest.len() + position;
            let mut depth = 0i32;
            for byte in &bytes[..absolute] {
                match byte {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    _ => {}
                }
            }
            if depth == 0 {
                return true;
            }
            rest = &rest[position + pattern.len()..];
        }
    }
    false
}

/// 对**单条断言**分类：返回它提供了哪些形态。
///
/// ⭐ R114：只看**这条断言自己的实参**，⛔ 不用固定字符窗口。
/// ⭐ **R118**：只有"**界定被遍历集合的大小**"的界才算数。
///
/// | 形态 | 界定集合大小？ |
/// |---|---|
/// | ① `x.len() >= N` / `> N` / `== N` | ✅ |
/// | ② `assert_eq!(x.len(), N)`（宏隐式相等） | ✅ |
/// | ③ `!x.is_empty()` | ✅（下界 1） |
/// | ⑤ 运行期计数器（**每轮无条件** `+= 1`）＋ 断言 | ✅（等价于下界 N） |
/// | ④ 聚合/值界 `assert_eq!(bpm_low, 16814)` | ⛔ **不界定集合**（求和可以为任何值） |
/// | ⑥ 接收者作用域的**子集计数** `assert_eq!(metric.hit_count(Kick), 2)` | ⛔ **不界定被遍历集合**（只界定过滤后的子集） |
fn classify_assertion(name: &str, args: &[String], locals: &[String]) -> u8 {
    let mut forms = 0u8;
    let first = args.first().map(|a| normalize(a)).unwrap_or_default();
    let joined = args
        .iter()
        .map(|a| normalize(a))
        .collect::<Vec<_>>()
        .join(",");
    if joined.contains("is_empty()") && joined.contains('!') {
        forms |= FORM_NOT_EMPTY;
    }
    if joined.contains(".len()") && (joined.contains(">=") || joined.contains('>')) {
        forms |= FORM_EXPLICIT_LEN;
    }
    if name == "assert_eq" && first.contains(".len()") && args.len() >= 2 {
        forms |= FORM_MACRO_LEN;
    }
    for local in locals {
        let mentions = |text: &str| {
            text.split(|c: char| !(c.is_alphanumeric() || c == '_'))
                .any(|word| word == local)
        };
        // ④ 聚合/值界：第一个实参是**裸标识符**且与常量比较（⛔ 不界定集合）。
        if name == "assert_eq" && mentions(&first) && args.len() >= 2 {
            let second = normalize(&args[1]);
            let is_literal =
                second.chars().all(|c| c.is_ascii_digit() || c == '_') || second.contains('*');
            let bare = !first.is_empty() && first.chars().all(|c| c.is_alphanumeric() || c == '_');
            if is_literal && bare {
                forms |= FORM_VALUE;
            }
        }
        // ⭐ R118 ⑤：计数器的断言读数。**是否算集合大小界**由调用方按
        // `unconditional_increment`（每轮无条件 `+= 1`）决定。
        if name == "assert" && mentions(&first) && (first.contains(">=") || first.contains('>')) {
            forms |= FORM_COUNTER;
        }
    }
    // ⑥ 接收者作用域的**子集计数**：第一个实参含方法调用、第二个是整数字面量
    //    （⛔ 只界定过滤后的子集，**不**界定被遍历集合）。
    if name == "assert_eq"
        && args.len() >= 2
        && first.contains('(')
        && first.contains('.')
        && !first.contains(".len()")
    {
        let second = normalize(&args[1]);
        if !second.is_empty() && second.chars().all(|c| c.is_ascii_digit() || c == '_') {
            forms |= FORM_SUBSET;
        }
    }
    forms
}

/// 生产代码之后的**测试区**（`#[cfg(test)]` 起）：R93 的对象是**判据**，不是生产代码。
fn test_region(raw: &str) -> &str {
    match raw.rfind("#[cfg(test)]") {
        Some(position) => &raw[position..],
        None => raw,
    }
}

/// 一段文本里**所有**断言提供的形态之并（R160 注册表检查用）。
fn forms_in_sample(text: &str) -> u8 {
    let mut all = 0u8;
    for name in [
        "assert_eq",
        "assert_ne",
        "assert",
        "prop_assert",
        "prop_assert_eq",
    ] {
        let needle = format!("{name}!(");
        let mut inner = text;
        while let Some(p) = inner.find(&needle) {
            let open_paren = p + needle.len() - 1;
            let args = top_level_args(inner, open_paren);
            all |= classify_assertion(name, &args, &mutable_locals(text));
            inner = &inner[open_paren + 1..];
        }
    }
    all
}

/// 掩码文本里的**测试函数体**区间（`#[test]` → 匹配的 `}`）。
fn test_bodies(masked: &str) -> Vec<(usize, usize)> {
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = masked[cursor..].find("#[test]") {
        let marker = cursor + relative;
        let Some(fn_pos) = masked[marker..].find("fn ") else {
            break;
        };
        let Some(open_rel) = masked[marker + fn_pos..].find('{') else {
            break;
        };
        let open = marker + fn_pos + open_rel;
        let mut depth = 1i32;
        let mut index = open + 1;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        out.push((open + 1, index.saturating_sub(1)));
        cursor = index;
        if cursor >= masked.len() {
            break;
        }
    }
    out
}

/// 一段文本里出现的"接收者"标识集合（被 `.` 或 `::` 跟着的标识）
fn receivers_in(expr: &str) -> Vec<String> {
    let chars: Vec<char> = expr.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut current = String::new();
    for (index, ch) in chars.iter().enumerate() {
        if ch.is_alphanumeric() || *ch == '_' {
            current.push(*ch);
        } else if !current.is_empty() {
            let next = chars.get(index + 1).copied();
            let prev_is_colon = index >= 2 && chars[index - 1] == ':' && chars[index - 2] == ':';
            if (next == Some('.') || (next == Some(':') && !prev_is_colon))
                && !out.contains(&current)
            {
                out.push(current.clone());
            }
            current.clear();
        }
    }
    // 基名（`&result.voicings` 的 `result` 已被上面收录；此处再兜一个基标识）
    let base: String = expr
        .trim_start_matches(['&', '(', ' '])
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if !base.is_empty() && !out.contains(&base) {
        out.push(base);
    }
    out
}

/// 扫描一段（已掩码的）源码：对每个"运行期根"循环，给出它是否被**根绑定**的界覆盖。
///
/// 返回 `(循环数, 根绑定数, 计数器/值界数, 无界清单)`。
///
/// ⭐ R114：**以测试函数体为作用域**（⛔ 不用固定字符窗口），并且
/// - 形态 ①②③ 必须由**提到本循环的接收者**的断言承担；
/// - 形态 ④/⑤ 必须由**在本循环体内被改动**的标识承担（改动点与断言点分开也算）。
fn scan_bounds(masked: &str) -> (usize, usize, usize, Vec<String>) {
    let dynamic_roots = [
        "GenreLibrary::all()",
        "GenreLibrary::ids()",
        "GenreLibrary::by_",
        ".hits()",
        ".notes()",
        ".voicings",
        ".movements",
    ];
    let bodies = test_bodies(masked);
    let bytes = masked.as_bytes();
    let mut loops = 0usize;
    let mut rooted = 0usize;
    let mut counter_only = 0usize;
    let mut unbounded: Vec<String> = Vec::new();
    let mut cursor = 0usize;
    while let Some(relative) = masked[cursor..].find("for ") {
        let loop_pos = cursor + relative;
        let after = &masked[loop_pos..];
        let Some(open) = after.find('{') else { break };
        let head = &after[..open];
        let Some(in_pos) = head.find(" in ") else {
            cursor = loop_pos + open + 1;
            continue;
        };
        let expr = head[in_pos + 4..].trim();
        if !dynamic_roots.iter().any(|root| expr.contains(root)) {
            cursor = loop_pos + open + 1;
            continue;
        }
        // 只在测试体内统计（R93 的对象是判据）
        let Some((scope_start, scope_end)) = bodies
            .iter()
            .copied()
            .filter(|(start, end)| *start <= loop_pos && loop_pos < *end)
            .min_by_key(|(start, end)| end - start)
        else {
            cursor = loop_pos + open + 1;
            continue;
        };
        loops += 1;
        let body_start = loop_pos + open + 1;
        let mut depth = 1i32;
        let mut index = body_start;
        while index < bytes.len() && depth > 0 {
            match bytes[index] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                _ => {}
            }
            index += 1;
        }
        let body_end = index.saturating_sub(1);
        let body = &masked[body_start..body_end];
        let scope = &masked[scope_start..scope_end];
        let receivers = receivers_in(expr);
        let locals = mutable_locals(&masked[scope_start..loop_pos]);
        // ⭐ R118 ⑤：只有"每轮**无条件**自增 1"的计数器才是集合大小的界；
        // 条件自增（在 `if` 里）或聚合（`+= n`）都不算。
        let mutated: Vec<String> = mutated_idents(body)
            .into_iter()
            .filter(|ident| locals.contains(ident) && unconditional_increment(body, ident))
            .collect();

        // 作用域内**所有**断言（一次性收集），再按"接收者/累加器"绑定到本循环
        let mut rooted_here = false;
        let mut counter_here = false;
        for name in [
            "assert_eq",
            "assert_ne",
            "assert",
            "prop_assert",
            "prop_assert_eq",
        ] {
            let needle = format!("{name}!(");
            let mut inner = scope;
            while let Some(p) = inner.find(&needle) {
                let open_paren = p + needle.len() - 1;
                let args = top_level_args(inner, open_paren);
                let forms = classify_assertion(name, &args, &locals);
                if forms != 0 {
                    let words: Vec<String> = args
                        .iter()
                        .flat_map(|arg| {
                            normalize(arg)
                                .split(|c: char| !(c.is_alphanumeric() || c == '_'))
                                .map(|word| word.to_owned())
                                .collect::<Vec<_>>()
                        })
                        .collect();
                    let receiver_mentioned =
                        receivers.iter().any(|receiver| words.contains(receiver));
                    // ⭐ R118：只有**界定集合大小**的形态（FORM_SET_SIZE 去掉计数器）能当"根绑定"。
                    // 计数器单独处理（要求每轮无条件 +1），所以这里排除 FORM_COUNTER。
                    if forms & (FORM_SET_SIZE & !FORM_COUNTER) != 0 && receiver_mentioned {
                        rooted_here = true;
                    }
                    if mutated.iter().any(|ident| words.contains(ident)) {
                        counter_here = true;
                    }
                }
                inner = &inner[open_paren + 1..];
            }
        }
        // 累加器的形态判定：作用域内提到"本循环改动的标识"的 assert_eq!/assert! 也算形态 ④/⑤
        if counter_here && !rooted_here {
            counter_only += 1;
        } else if rooted_here {
            rooted += 1;
        } else {
            unbounded.push(format!("for ... in {expr}"));
        }
        cursor = body_start;
    }
    (loops, rooted, counter_only, unbounded)
}

// ---------------------------------------------------------------------------
// 判据 1（R115）：常驻的"界"扫描器 ＋ R56/R112 正负对照
// ---------------------------------------------------------------------------

/// ⭐ 把"界的五种形态"做成**常驻判据**，并**自带正负对照**（R56/R112）。
#[test]
fn scan_loops_are_bound_and_the_classifier_has_positive_and_negative_controls() {
    // ---- R113：掩码逐字节等长（含多字节 UTF-8 与三类上下文）----
    for sample in [
        "let s = \"中文 ♯\"; assert_eq!(a, a);",
        "// 注释 中文\nassert_ne!(x, x);",
        "/* 块注释 ♭♯ */ code();",
        "",
    ] {
        let masked = mask_preserving_len(sample);
        assert_eq!(masked.len(), sample.len(), "R113 byte-length: {sample:?}");
    }

    // ---- R56/R112：五种形态**每种一条已知绿** ＋ **一条"无界"已知红** ----
    // (label, sample, expect_bound, expected form mask)
    let cases: [(&str, &str, bool, u8); 9] = [
        // ① 显式 len() >= N（界定集合 ✅）
        (
            "grid",
            "for hit in grid.hits() { assert!(grid.hits().len() >= 2); }",
            true,
            FORM_EXPLICIT_LEN,
        ),
        // ② 宏隐式相等（界定集合 ✅）
        (
            "all",
            "for rule in GenreLibrary::all() { assert_eq!(GenreLibrary::all().len(), 182); }",
            true,
            FORM_MACRO_LEN,
        ),
        // ③ !is_empty()（界定集合 ✅）
        (
            "grid",
            "for hit in grid.hits() { assert!(!grid.hits().is_empty()); }",
            true,
            FORM_NOT_EMPTY,
        ),
        // ⑤ 运行期计数器：每轮**无条件** += 1（界定集合 ✅）
        (
            "all",
            "let mut seen = 0usize; for rule in GenreLibrary::all() { seen += 1; } assert!(seen >= 182);",
            true,
            FORM_COUNTER,
        ),
        // ⛔ ④ 聚合/值界（R118 已知红：**不界定集合**）
        (
            "all",
            "let mut bpm = 0u32; for rule in GenreLibrary::all() { bpm += rule.default_bpm_range.0 as u32; } assert_eq!(bpm, 16814);",
            false,
            FORM_VALUE,
        ),
        // ⛔ ⑤′ 计数器但**条件自增**（R118 已知红）
        (
            "all",
            "let mut seen = 0usize; for rule in GenreLibrary::all() { if rule.swing.is_some() { seen += 1; } } assert!(seen >= 1);",
            false,
            FORM_COUNTER,
        ),
        // ⛔ ⑥ 接收者作用域的**子集计数**（R118 已知红）
        (
            "pattern",
            "for hit in pattern.hits() { } assert_eq!(pattern.hit_count(2), 4);",
            false,
            FORM_SUBSET,
        ),
        // ⛔ 已知红（R112 的正对照）：界**看起来像**形态 ①，但它绑在别的接收者
        //    （`rule.id` 而不是被遍历的 `GenreLibrary::all()`）⇒ 判定必须是**无界**。
        (
            "all",
            "for rule in GenreLibrary::all() { assert!(rule.id.len() > 0); }",
            false,
            FORM_EXPLICIT_LEN,
        ),
        // ⛔ R119 near-miss：循环遍历 gridlines.hits()，界却写在 grid.hits() 上
        (
            "gridlines",
            "for hit in gridlines.hits() { assert!(grid.hits().len() >= 2); }",
            false,
            FORM_EXPLICIT_LEN,
        ),
    ];
    let mut green_seen = 0usize;
    let mut red_seen = 0usize;
    let mut covered_forms = 0u8; // R160 方向②：注册形态必须**被正对照命中**
    let mut declared_forms = 0u8; // R160 方向①：正对照只能命中**已登记**形态
    for (label, sample, expect_bound, expected_form) in cases {
        // 样例外层套一个合成测试体 —— 扫描器的作用域就是"测试函数体"。
        let wrapped = format!("#[test]\nfn sample() {{\n{sample}\n}}\n");
        let masked = mask_preserving_len(&wrapped);
        let (loops, rooted, counter_only, unbounded) = scan_bounds(&masked);
        assert_eq!(
            loops, 1,
            "{label}: exactly one runtime loop must be recognised"
        );
        // ⭐ R160 双向归零：
        // ① 正对照命中的形态**必须恰好**是它登记的形态（既不多也不少）；
        // ② 注册表里的**每一种**形态都必须被某个正对照命中（没有"有形态没人证明"）。
        let hit_forms = forms_in_sample(&wrapped);
        assert_eq!(
            hit_forms, expected_form,
            "{label}: classified forms {hit_forms:#07b} != registered form {expected_form:#07b}"
        );
        assert_eq!(
            hit_forms & !FORM_REGISTRY.iter().fold(0u8, |acc, (flag, _)| acc | flag),
            0,
            "{label}: hits a form that is not in the registry"
        );
        // ⭐ R160：覆盖统计对**所有**对照生效 —— ④/⑥ 是**已登记但不定界**的形态，
        // 它们的正对照就是"必须判成无界"（expect_bound = false）。
        covered_forms |= hit_forms;
        declared_forms |= expected_form;
        let bounded = rooted + counter_only == 1 && unbounded.is_empty();
        assert_eq!(
            bounded, expect_bound,
            "{label}: classifier verdict {bounded} (rooted={rooted} counter_only={counter_only} unbounded={unbounded:?})"
        );
        if expect_bound {
            green_seen += 1;
        } else {
            red_seen += 1;
        }
    }
    assert_eq!(green_seen, 4, "four known-green set-size forms");
    assert_eq!(
        red_seen, 5,
        "five known-red samples (R118 value/subset/conditional + R119 near-miss + no bound)"
    );
    // ⭐ R160 双向归零（两条一起才能同时排除"有形态没人证明"与"注册了不存在的形态"）。
    let registry = FORM_REGISTRY.iter().fold(0u8, |acc, (flag, _)| acc | flag);
    assert_eq!(registry.count_ones(), 6, "six registered forms");
    assert_eq!(
        covered_forms, registry,
        "R160①: every registered form must be hit by a positive control"
    );
    assert_eq!(
        declared_forms, registry,
        "R160②: positive controls may only declare registered forms"
    );
    assert_eq!(
        FORM_SET_SIZE | FORM_VALUE | FORM_SUBSET,
        registry,
        "allowlist partition"
    );

    // ---- 真源码：15 个文件 ----
    const SOURCES: [(&str, &str); 15] = [
        ("src/lib.rs", include_str!("../src/lib.rs")),
        ("src/error.rs", include_str!("../src/error.rs")),
        ("src/pitch.rs", include_str!("../src/pitch.rs")),
        ("src/scale.rs", include_str!("../src/scale.rs")),
        ("src/chord.rs", include_str!("../src/chord.rs")),
        ("src/progression.rs", include_str!("../src/progression.rs")),
        (
            "src/voice_leading.rs",
            include_str!("../src/voice_leading.rs"),
        ),
        ("src/genre.rs", include_str!("../src/genre.rs")),
        ("src/swing.rs", include_str!("../src/swing.rs")),
        ("src/rhythm.rs", include_str!("../src/rhythm.rs")),
        ("src/melody.rs", include_str!("../src/melody.rs")),
        ("src/drum.rs", include_str!("../src/drum.rs")),
        ("tests/properties.rs", include_str!("properties.rs")),
        ("tests/golden_tables.rs", include_str!("golden_tables.rs")),
        (
            "tests/determinism_digests.rs",
            include_str!("determinism_digests.rs"),
        ),
    ];
    let mut total_loops = 0usize;
    let mut total_rooted = 0usize;
    let mut total_counter = 0usize;
    let mut residual: Vec<(String, String)> = Vec::new();
    for (name, raw) in SOURCES {
        let region = test_region(raw);
        let masked = mask_preserving_len(region);
        assert_eq!(masked.len(), region.len(), "R113: {name}");
        let (loops, rooted, counter_only, unbounded) = scan_bounds(&masked);
        total_loops += loops;
        total_rooted += rooted;
        total_counter += counter_only;
        for item in unbounded {
            residual.push((name.to_owned(), item));
        }
    }
    // ⭐ R93 非真空：扫描域必须达到下界（数字是实测值，改动即红）。
    assert_eq!(
        total_loops, 76,
        "runtime-collection loops in the criteria corpus"
    );
    assert_eq!(
        total_rooted, 52,
        "loops covered by a receiver-bound assertion"
    );
    assert_eq!(
        total_counter, 1,
        "loops covered by a mutated-accumulator bound"
    );
    // ⭐ **残余清单**（R97 的形态）：R118 收紧后分类器**不认**这 **23** 个循环。
    // 逐**条**钉住（文件 ＋ 循环表达式）⇒ 新增一条、删掉一条、或换一个循环都会红。
    // 每一行的"为什么认不到"记在 `docs/shape-d-audit.md` §1j。
    let golden_residual: [(&str, &str); 23] = [
        (
            "src/voice_leading.rs",
            "for ... in spans.iter().zip(result.voicings.iter())",
        ),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        (
            "src/genre.rs",
            "for ... in GenreLibrary::by_drum_style(DrumStyle::Metric)",
        ),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        ("src/genre.rs", "for ... in GenreLibrary::all()"),
        ("src/melody.rs", "for ... in GenreLibrary::all()"),
        (
            "src/drum.rs",
            "for ... in metric .hits() .iter() .filter(|hit| hit.voice == DrumVoice::Kick)",
        ),
        (
            "src/drum.rs",
            "for ... in metric .hits() .iter() .filter(|hit| hit.voice == DrumVoice::Kick)",
        ),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        ("tests/properties.rs", "for ... in grid.hits()"),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        ("tests/properties.rs", "for ... in melody.notes()"),
        ("tests/properties.rs", "for ... in grid.hits()"),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        (
            "tests/properties.rs",
            "for ... in pattern.hits().windows(2)",
        ),
        ("tests/properties.rs", "for ... in pattern.hits()"),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        ("tests/properties.rs", "for ... in GenreLibrary::all()"),
        (
            "tests/properties.rs",
            "for ... in metric .hits() .iter() .filter(|hit| hit.voice == DrumVoice::Kick)",
        ),
    ];
    let mut expected: Vec<(String, String)> = golden_residual
        .iter()
        .map(|(file, expr)| ((*file).to_owned(), (*expr).to_owned()))
        .collect();
    expected.sort();
    // 多行循环表达式在比较前归一化空白（两侧都归一化）。
    let mut actual: Vec<(String, String)> = residual
        .iter()
        .map(|(file, expr)| (file.clone(), normalize(expr)))
        .collect();
    actual.sort();
    assert_eq!(
        actual, expected,
        "the unrecognised-bound residual changed; actual: {actual:#?}"
    );
    assert_eq!(actual.len(), 23, "23 registered unrecognised loops");
}

// ---------------------------------------------------------------------------
// 判据 2（R100/R116）：判据不得读运行期外部资源
// ---------------------------------------------------------------------------

/// ⭐ R100/R116：**判据**（`src/**` 的测试模块 ＋ `tests/**`）不得读运行期外部资源。
///
/// 动机：外部夹具缺失是一种**真空形态** —— 读不到文件就 `SKIP`，判据静默失去判别力。
/// 本 crate 的选择是**判据只用纯内存数据**；本判据把这条选择变成可执行的检查。
///
/// ⚠ 与 `src/lib.rs::crate_has_no_hidden_nondeterminism_sources` 不重叠：
/// 那条只看**生产代码**（到 `#[cfg(test)]` 为止），测试代码故意豁免。
#[test]
fn no_criterion_reads_a_runtime_external_resource() {
    // R56：先喂"一条已知红 ＋ 一条已知绿"。
    let forbidden = [
        "std::fs::",
        "std::env::var",
        "std::env::var_os",
        "read_dir",
        "File::open",
        "CARGO_TARGET_TMPDIR",
        "CARGO_MANIFEST_DIR",
        "current_dir",
        "tempfile",
        "std::process::Command",
    ];
    let hits_in = |text: &str| -> Vec<String> {
        let masked = mask_preserving_len(text);
        let mut found = Vec::new();
        for (line_number, line) in masked.lines().enumerate() {
            for needle in forbidden {
                if line.contains(needle) {
                    found.push(format!("line {}: {needle}", line_number + 1));
                }
            }
        }
        found
    };
    assert!(
        !hits_in("let text = std::fs::read_to_string(p).unwrap();").is_empty(),
        "known-red: std::fs:: must be detected"
    );
    assert!(
        hits_in("let text = String::from(\"std::fs::read_to_string\");").is_empty(),
        "known-green: the needle inside a string literal is masked"
    );
    assert!(
        hits_in("// std::fs::read_to_string\nlet x = 1;").is_empty(),
        "known-green: the needle inside a comment is masked"
    );

    // 真源码：12 个生产文件 ＋ 3 个集成测试文件（测试模块在 `src` 文件里）。
    const SOURCES: [(&str, &str); 15] = [
        ("src/lib.rs", include_str!("../src/lib.rs")),
        ("src/error.rs", include_str!("../src/error.rs")),
        ("src/pitch.rs", include_str!("../src/pitch.rs")),
        ("src/scale.rs", include_str!("../src/scale.rs")),
        ("src/chord.rs", include_str!("../src/chord.rs")),
        ("src/progression.rs", include_str!("../src/progression.rs")),
        (
            "src/voice_leading.rs",
            include_str!("../src/voice_leading.rs"),
        ),
        ("src/genre.rs", include_str!("../src/genre.rs")),
        ("src/swing.rs", include_str!("../src/swing.rs")),
        ("src/rhythm.rs", include_str!("../src/rhythm.rs")),
        ("src/melody.rs", include_str!("../src/melody.rs")),
        ("src/drum.rs", include_str!("../src/drum.rs")),
        ("tests/properties.rs", include_str!("properties.rs")),
        ("tests/golden_tables.rs", include_str!("golden_tables.rs")),
        (
            "tests/determinism_digests.rs",
            include_str!("determinism_digests.rs"),
        ),
    ];
    let mut scanned_files = 0usize;
    let mut scanned_lines = 0usize;
    let mut violations: Vec<String> = Vec::new();
    for (name, raw) in SOURCES {
        scanned_files += 1;
        scanned_lines += raw.lines().count();
        for hit in hits_in(raw) {
            // 本判据自己的禁用针清单是字面量（未掩码时可见），掩码后不可见 ⇒
            // 真代码里的命中只可能来自**未掩码**的调用。
            violations.push(format!("{name}:{hit}"));
        }
    }
    // ⭐ R93 非真空：扫描域必须达到下界。
    assert_eq!(scanned_files, 15, "scanned files");
    assert!(scanned_lines >= 8000, "scanned lines: {scanned_lines}");
    assert!(
        violations.is_empty(),
        "criteria must not read runtime external resources: {violations:#?}"
    );

    // R116：附一条**可执行的查证命令**的对应读数（在报告里给命令与输出）。
    // 这里把"读磁盘的判据"这一陈述锚在代码上：本 crate 的判据只吃纯内存数据。
    let key = Scale::new(PitchClass::C, ScaleKind::Major);
    let spans = expand_progression(&key, "I-V", 2).unwrap();
    let grid = metric_grid(Meter::COMMON, 2, 4).unwrap();
    let melody = melody_over_chords(&key, &spans, &grid, MelodyConstraints::DEFAULT, 1).unwrap();
    assert!(!melody.notes().is_empty());
    assert_eq!(GenreLibrary::by_source(SOURCE_TRADITIONAL_THEORY).len(), 52);
}
