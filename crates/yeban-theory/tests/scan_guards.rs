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
// ⑥ = `assert_eq!(<提到本循环接收者的表达式>, <整数字面量>)`，在 scan_bounds 里就地判定。

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

/// 对**单条断言**分类：返回它提供了哪些形态。
///
/// ⭐ R114：只看**这条断言自己的实参**，⛔ 不用固定字符窗口。
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
        if name == "assert_eq" && mentions(&first) && args.len() >= 2 {
            let second = normalize(&args[1]);
            if second.chars().all(|c| c.is_ascii_digit() || c == '_') || second.contains('*') {
                forms |= FORM_VALUE | FORM_COUNTER;
            }
        }
        if name == "assert" && mentions(&first) && (first.contains(">=") || first.contains('>')) {
            forms |= FORM_COUNTER;
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
        let mutated: Vec<String> = mutated_idents(body)
            .into_iter()
            .filter(|ident| locals.contains(ident))
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
                    // 形态 ①/②/③
                    if forms & (FORM_EXPLICIT_LEN | FORM_MACRO_LEN | FORM_NOT_EMPTY) != 0
                        && receiver_mentioned
                    {
                        rooted_here = true;
                    }
                    // 形态 ⑥（本批新增）：`assert_eq!(<提到本循环接收者的表达式>, <整数字面量>)`
                    // 例如 `assert_eq!(metric.hit_count(DrumVoice::Kick), 2)` —— 这也是真正的域下界。
                    if name == "assert_eq" && receiver_mentioned && args.len() >= 2 {
                        let second = normalize(&args[1]);
                        if !second.is_empty()
                            && second
                                .chars()
                                .all(|c| c.is_ascii_digit() || c == '_' || c == ' ')
                        {
                            rooted_here = true;
                        }
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
    let cases: [(&str, &str, bool); 6] = [
        // ① 显式 len() >= N
        (
            "grid",
            "for hit in grid.hits() { assert!(grid.hits().len() >= 2); }",
            true,
        ),
        // ② 宏隐式相等
        (
            "all",
            "for rule in GenreLibrary::all() { assert_eq!(GenreLibrary::all().len(), 182); }",
            true,
        ),
        // ③ !is_empty()
        (
            "grid",
            "for hit in grid.hits() { assert!(!grid.hits().is_empty()); }",
            true,
        ),
        // ④ 值界（累加器 == 常量）
        (
            "all",
            "let mut total = 0usize; for rule in GenreLibrary::all() { total += 1; } assert_eq!(total, 16814);",
            true,
        ),
        // ⑤ 运行期计数器
        (
            "all",
            "let mut seen = 0usize; for rule in GenreLibrary::all() { seen += 1; } assert!(seen >= 182);",
            true,
        ),
        // ⛔ 已知红：无任何界（R112 的正对照：这条**必须**被判成无界）
        (
            "all",
            "for rule in GenreLibrary::all() { assert!(rule.id.len() > 0); }",
            false,
        ),
    ];
    let mut green_seen = 0usize;
    let mut red_seen = 0usize;
    for (label, sample, expect_bound) in cases {
        // 样例外层套一个合成测试体 —— 扫描器的作用域就是"测试函数体"。
        let wrapped = format!("#[test]\nfn sample() {{\n{sample}\n}}\n");
        let masked = mask_preserving_len(&wrapped);
        let (loops, rooted, counter_only, unbounded) = scan_bounds(&masked);
        assert_eq!(
            loops, 1,
            "{label}: exactly one runtime loop must be recognised"
        );
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
    assert_eq!(green_seen, 5, "five known-green forms");
    assert_eq!(red_seen, 1, "one known-red unbounded sample");

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
        total_counter, 9,
        "loops covered by a mutated-accumulator bound"
    );
    // ⭐ **残余清单**（R97 的形态）：分类器**不认**这 **15** 个循环的界形态，
    // 但它们各自的界已由人工核对（见 `docs/shape-d-audit.md` §1f）。
    // 逐文件钉住计数 ⇒ 新增一个"分类器不认"的循环会让这里变红。
    let golden: [(&str, usize); 4] = [
        ("src/genre.rs", 3),
        ("src/voice_leading.rs", 1),
        ("src/drum.rs", 2),
        ("tests/properties.rs", 9),
    ];
    let mut actual: Vec<(String, usize)> = Vec::new();
    for (name, _) in &residual {
        match actual.iter_mut().find(|(key, _)| key == name) {
            Some((_, count)) => *count += 1,
            None => actual.push((name.clone(), 1)),
        }
    }
    actual.sort();
    let mut expected: Vec<(String, usize)> = golden
        .iter()
        .map(|(name, count)| ((*name).to_owned(), *count))
        .collect();
    expected.sort();
    assert_eq!(
        actual, expected,
        "the unrecognised-bound residual changed; residual items: {residual:#?}"
    );
    assert_eq!(
        residual.len(),
        golden.iter().map(|(_, count)| count).sum::<usize>(),
        "residual total"
    );
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
