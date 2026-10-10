//! 常驻判据（裁决 **R115**：一次性审计 ⛔ 不等于判据）。
//!
//! 本文件把第十七批的**一次性普查**落成**进门禁的判据**：本 crate 的 `src/**` 里
//! **不允许**出现「无界的量词断言」—— 也就是「遍历／量化某个集合，却没有任何
//! 迹象表明这个集合非空」的 `assert!`。
//!
//! 为什么这不是洁癖：`x.iter().all(..)` 在**空集合**上恒为 `true`、
//! `!x.iter().any(..)` 在空集合上同样恒为 `true` ⇒ 这类断言会在集合被清空、
//! 路径被过滤、平台差异导致集合为空时**静默通过**（R93／R102／R109）。
//!
//! ## 认的下界形态（裁决 R111 的五种 ＋ R112 的正对照 ＋ 字面量集合）
//! ① 显式 `x.len() >= N`／`> N`　② 宏隐式相等 `assert_eq!(x.len(), N)`
//! ③ 非空 `!x.is_empty()`　④ 值界（与非空字面量比较）　⑤ 运行期计数器
//! ⑥ 正对照（同一谓词在**另一个实例**上证明能命中，R112）
//! ⑦ 被遍历的集合本身是**字面量数组／固定范围／全大写常量**（按构造非真空）
//!
//! ## 已登记的局限（如实，⛔ 不假装能核）
//! - 文本扫描器**无法**核实 ⑥ 的正对照与目标**同类型**（R104 同族）。
//! - 本判据只覆盖 `assert!` 里**直接出现**的 `.all(`／`.any(` 形态；把量词结果先存进
//!   变量再断言（`let ok = x.iter().all(..); assert!(ok);`）**绕过**本判据（R104）。
//! - 只扫 `src/**`（`tests/**` 由同族判据在各自的库里覆盖）。

use std::fs;

/// 逐**字节**掩码注释与字符串字面量（R113：掩码与原文字节等长 ⇒ 偏移可直接映射）。
fn mask(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    #[derive(PartialEq)]
    enum State {
        Code,
        Line,
        Block,
        Str,
    }
    let mut state = State::Code;
    while i < bytes.len() {
        let c = bytes[i];
        let next = if i + 1 < bytes.len() { bytes[i + 1] } else { 0 };
        match state {
            State::Code => {
                if c == b'/' && next == b'/' {
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    state = State::Line;
                    i += 2;
                } else if c == b'/' && next == b'*' {
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    state = State::Block;
                    i += 2;
                } else if c == b'"' {
                    state = State::Str;
                    i += 1;
                } else {
                    i += 1;
                }
            }
            State::Line => {
                if c == b'\n' {
                    state = State::Code;
                } else {
                    out[i] = b' ';
                }
                i += 1;
            }
            State::Block => {
                if c == b'*' && next == b'/' {
                    out[i] = b' ';
                    out[i + 1] = b' ';
                    state = State::Code;
                    i += 2;
                } else {
                    if c != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
            State::Str => {
                if c == b'\\' {
                    out[i] = b' ';
                    if i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                } else if c == b'"' {
                    state = State::Code;
                    i += 1;
                } else {
                    if c != b'\n' {
                        out[i] = b' ';
                    }
                    i += 1;
                }
            }
        }
    }
    String::from_utf8(out).expect("the mask is byte-wise and keeps ASCII/UTF-8 shape")
}

/// 量词断言里**被量化集合的根绑定**（`v.iter().all(..)` ⇒ `v`；`!c.iter().any(..)` ⇒ `c`）。
fn quantified_root(body: &str) -> Option<String> {
    let mut rest = body;
    while let Some(index) = rest.find("assert!(") {
        let tail = &rest[index..];
        let end = tail.find(");").map(|e| e + 1).unwrap_or(tail.len());
        let call = &tail[..end.min(tail.len())];
        let after = call["assert!(".len()..].trim_start();
        let quantified =
            call.contains(".all(") || (after.starts_with('!') && call.contains(".any("));
        if quantified {
            // 接收者链的**根**：`v.iter().all(` ⇒ `v`；`instrument.warnings().iter().all(` ⇒ `instrument`。
            let receiver = after.trim_start_matches('!').trim_start();
            let root: String = receiver
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !root.is_empty() {
                return Some(root);
            }
        }
        rest = &tail[2..];
    }
    None
}

/// 把函数体粗切成语句（`;` 与花括号）。
fn statements(body: &str) -> Vec<&str> {
    body.split([';', '{', '}']).collect()
}

/// 7 种「非真空」迹象里有没有任何一种，**且必须与根绑定相关**（R114：⛔ 不许借用邻居）。
///
/// `root == None` 表示本体内根本没有量词断言 ⇒ 只按"有没有字面量／固定范围循环"判定。
fn has_non_vacuity_evidence(body: &str, root: Option<&str>) -> bool {
    let mentions_root = |stmt: &str| root.is_none_or(|r| stmt.contains(r));
    for stmt in statements(body) {
        if !mentions_root(stmt) {
            continue;
        }
        // ① 显式 ② 宏隐式相等 ③ 非空 ④ 值界 ⑤ 运行期计数器
        if stmt.contains(".len() >=")
            || stmt.contains(".len() > ")
            || stmt.contains(".count() >=")
            || stmt.contains(".count() > ")
            // ② 宏隐式相等：**两种实参顺序都认**（`assert_eq!(x.len(), N)` 与 `assert_eq!(N, x.len())`）。
            // ⚠️ 必须同时要求 `assert_eq!(`：否则 `let _ = x.len();` 这种**没有下界**的语句也会被认成下界
            // （实测：Z1 把实参换序后曾被误判为"无下界" ⇒ 假阳性；A1 删掉下界后必须仍判红）。
            || (stmt.contains("assert_eq!(") && stmt.contains(".len()"))
            || stmt.contains("!.is_empty()")
            || non_empty_call(stmt)
            || has_value_bound(stmt)
            || [
                "seen", "count", "total", "scanned", "checked", "visited", "hits",
            ]
            .iter()
            .any(|n| stmt.contains(&format!("{n} >=")) || stmt.contains(&format!("{n} > ")))
        {
            return true;
        }
    }
    // ⑤ 运行期计数器：计数器语句本身不含根，但**循环必须遍历这个根**
    if root.is_some_and(|r| iterates(body, r)) && counter_bound(body) {
        return true;
    }
    // ⑥ 正对照（R112）：正极性 `any`。⚠️ 登记局限：文本层核不到"同类型"。
    if positive_any(body) {
        return true;
    }
    // ⑦ 字面量集合：⛔ 必须**遍历这个根**（否则就是"借用邻居"）
    match root {
        Some(r) => iterates_literal(body, r),
        None => literal_iteration(body),
    }
}

/// 有没有 `for <pat> in <expr>` 且 `expr` 的根是 `root`。
fn iterates(body: &str, root: &str) -> bool {
    let mut rest = body;
    while let Some(index) = rest.find("for ") {
        if let Some(rel) = rest[index..].find(" in ") {
            let after = rest[index + rel + " in ".len()..].trim_start();
            if after.trim_start_matches('&').starts_with(root) {
                return true;
            }
        }
        rest = &rest[index + 4..];
    }
    false
}

/// 有没有 `for <pat> in <字面量/范围/常量>`，且**该迭代表达式的根是 `root`**。
fn iterates_literal(body: &str, root: &str) -> bool {
    let mut rest = body;
    while let Some(index) = rest.find("for ") {
        if let Some(rel) = rest[index..].find(" in ") {
            let after = rest[index + rel + " in ".len()..].trim_start();
            let head: String = after
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '{')
                .collect();
            let bare = head.trim_start_matches('&');
            let first = bare.chars().next();
            let looks_literal = bare.starts_with('[')
                || bare.starts_with("vec![")
                || first.is_some_and(|c| c.is_ascii_digit() || c == '-')
                || first.is_some_and(|c| c.is_ascii_uppercase());
            if looks_literal && bare.contains(root) {
                return true;
            }
        }
        rest = &rest[index + 4..];
    }
    false
}

/// 运行期计数器下界（`assert!(checked >= 3)` 之类）。
fn counter_bound(body: &str) -> bool {
    [
        "seen", "count", "total", "scanned", "checked", "visited", "hits",
    ]
    .iter()
    .any(|n| body.contains(&format!("{n} >=")) || body.contains(&format!("{n} > ")))
}

/// `!x.is_empty()`（含 `!x.iter().is_empty()` 之外的常见写法）。
fn non_empty_call(body: &str) -> bool {
    let bytes = body.as_bytes();
    for (index, _) in body.match_indices("is_empty()") {
        let mut start = index;
        while start > 0
            && bytes[start - 1] != b'!'
            && bytes[start - 1] != b'\n'
            && bytes[start - 1] != b' '
        {
            start -= 1;
        }
        if start > 0 && bytes[start - 1] == b'!' {
            return true;
        }
        if index >= 1 && bytes[index - 1] == b'!' {
            return true;
        }
    }
    false
}

/// 值界：`assert_eq!(x, [..])` / `assert_eq!(x, vec![..])` 且字面量非空。
fn has_value_bound(body: &str) -> bool {
    for marker in ["assert_eq!(", "assert_eq!(\n"] {
        let mut rest = body;
        while let Some(index) = rest.find(marker) {
            let tail = &rest[index..];
            let end = tail.find(");").map(|e| e + 1).unwrap_or(tail.len());
            let call = &tail[..end.min(tail.len())];
            let holds_literal = (call.contains('[') && call.contains(']'))
                || (call.contains("vec![") && call.contains(']'));
            if holds_literal && !call.contains("is_empty") {
                let inner = call.split_once(',').map(|(_, rhs)| rhs).unwrap_or("");
                if inner.contains(']') && inner.chars().any(|c| c.is_ascii_digit() || c == '"') {
                    return true;
                }
            }
            rest = &tail[2..];
        }
    }
    false
}

/// ⑦ 字面量集合：`for <pat> in <expr>` 且 **`<expr>` 本身**是字面量数组／固定范围／全大写常量。
///
/// ⚠️ 必须**锚定到 `for` 的迭代表达式**：早先写成"body 里含 `..`"⇒ 几乎每个测试体
/// 都因别处的 `0..3` 被判为"非真空" ⇒ 判据**形同虚设**（实测：删掉一处的 `len()` 下界后
/// 判据仍全绿）。这正是 R108「每条分类路径都要喂已知红」的实例。
fn literal_iteration(body: &str) -> bool {
    let mut rest = body;
    while let Some(index) = rest.find("for ") {
        if let Some(rel) = rest[index..].find(" in ") {
            let after = rest[index + rel + " in ".len()..].trim_start();
            let head: String = after
                .chars()
                .take_while(|c| !c.is_whitespace() && *c != '{')
                .collect();
            let first = head.chars().next();
            if head.starts_with('[')
                || head.starts_with("vec![")
                || first.is_some_and(|c| c.is_ascii_digit() || c == '-')
                || first.is_some_and(|c| c.is_ascii_uppercase())
            {
                return true;
            }
        }
        rest = &rest[index + 4..];
    }
    false
}

/// 正对照：存在**正极性**的 `assert!(… any(`（R112）。
fn positive_any(body: &str) -> bool {
    let mut rest = body;
    while let Some(index) = rest.find("assert!(") {
        let tail = &rest[index..];
        let end = tail.find(");").map(|e| e + 1).unwrap_or(tail.len());
        let call = &tail[..end.min(tail.len())];
        let after = call["assert!(".len()..].trim_start();
        if call.contains(".any(") && !after.starts_with('!') {
            return true;
        }
        rest = &tail[2..];
    }
    false
}

/// 把源码切成 `(函数名, 函数体)`（按花括号配对）。
fn functions(source: &str) -> Vec<(String, String)> {
    let masked = mask(source);
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = masked[search..].find("fn ") {
        let start = search + rel;
        let name_start = start + 3;
        let name_end = masked[name_start..]
            .find('(')
            .map(|e| name_start + e)
            .unwrap_or(name_start);
        let name = masked[name_start..name_end].trim().to_string();
        let Some(brace) = masked[name_end..].find('{').map(|e| name_end + e) else {
            break;
        };
        let mut depth = 0i32;
        let mut end = brace;
        let mut i = brace;
        while i < bytes.len() {
            if bytes[i] == b'{' {
                depth += 1;
            } else if bytes[i] == b'}' {
                depth -= 1;
                if depth == 0 {
                    end = i + 1;
                    break;
                }
            }
            i += 1;
        }
        if end > brace {
            out.push((name, source[brace..end].to_string()));
        }
        search = end.max(name_end + 1);
    }
    out
}

/// 量化断言：`assert!( … .all(` 或 `assert!( ! … .any(`（两类真空面）。
fn is_quantified_assertion(body: &str) -> bool {
    let mut rest = body;
    while let Some(index) = rest.find("assert!(") {
        let tail = &rest[index..];
        let end = tail.find(");").map(|e| e + 1).unwrap_or(tail.len());
        let call = &tail[..end.min(tail.len())];
        if call.contains(".all(") {
            return true;
        }
        let after = call["assert!(".len()..].trim_start();
        if after.starts_with('!') && call.contains(".any(") {
            return true;
        }
        rest = &tail[2..];
    }
    false
}

/// R56／R108：先给分类器喂**两条已知绿 ＋ 两条已知红**，两条都判对才允许上真数据。
fn self_test_classifier() {
    let greens = [
        // ① 显式下界
        "let v = f(); assert!(v.len() >= 3); assert!(v.iter().all(|x| *x > 0));",
        // ② 宏隐式相等
        "let v = f(); assert_eq!(v.len(), 4); assert!(v.iter().all(|x| *x > 0));",
        // ③ 非空
        "let v = f(); assert!(!v.is_empty()); assert!(v.iter().all(|x| *x > 0));",
        // ④ 值界
        "let v = f(); assert_eq!(v, vec![1, 2, 3]); assert!(v.iter().all(|x| *x > 0));",
        // ⑤ 计数器
        "let v = f(); let mut checked = 0; for x in &v { checked += 1; } assert!(checked >= 3); assert!(v.iter().all(|x| *x > 0));",
        // ⑥ 正对照
        "let v = f(); assert!(!v.iter().any(|x| *x > 0)); let c = g(); assert!(c.iter().any(|x| *x > 0));",
        // ⑦ 字面量集合
        "for x in [1, 2, 3] { assert!(x > 0); }",
        // ② 的**实参换序**（必须同样被接受）
        "let v = f(); assert_eq!(4, v.len()); assert!(v.iter().all(|x| *x > 0));",
    ];
    let reds = [
        "let v = f(); assert!(v.iter().all(|x| *x > 0));",
        "let v = f(); assert!(!v.iter().any(|x| *x > 0));",
        // ⚠️ 专打 ⑦ 的过宽：别处有 `..`，但被量化的集合**不是**字面量
        "let v = f(); for i in 0..3 { let _ = i; } assert!(v.iter().all(|x| *x > 0));",
        // ⚠️ 同上：别处有 `for … in [..]`，但被量化的集合不是它
        "let v = f(); let w = [1, 2]; for x in w { let _ = x; } assert!(v.iter().all(|x| *x > 0));",
    ];
    for (index, body) in greens.iter().enumerate() {
        // 绿夹具只要求「被判为非真空」（⑦ 那种是**循环**而不是量词断言，不适用量化检查）。
        assert!(
            has_non_vacuity_evidence(body, quantified_root(body).as_deref()),
            "green {index} must be accepted as non-vacuous: {body}"
        );
    }
    for (index, body) in reds.iter().enumerate() {
        assert!(
            is_quantified_assertion(body),
            "red {index} must be quantified"
        );
        assert!(
            !has_non_vacuity_evidence(body, quantified_root(body).as_deref()),
            "red {index} must be rejected: {body}"
        );
    }
    // R113：掩码逐字节等长（含多字节内容）
    let raw = "// 中文注释 assert_ne!(1, 2)\nlet a = \"中文\";\nlet b = 3;\n";
    assert_eq!(mask(raw).len(), raw.len(), "the mask must be byte-wise");
    assert!(
        !mask(raw).contains("assert_ne!(1, 2)"),
        "comment decoy must be masked"
    );
}

#[test]
fn no_unbounded_all_any_assertion_in_this_crate() {
    self_test_classifier();

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&dir).expect("read src/") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            let text = fs::read_to_string(&path).expect("read source");
            sources.push((path, text));
        }
    }
    // ⭐ R93：先钉住被扫集合**非空且达下界** —— 否则本判据会随着"文件没读到"真空通过。
    assert!(
        sources.len() >= 8,
        "R93: the source scan must not go empty, got {}",
        sources.len()
    );

    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for (path, text) in &sources {
        for (name, body) in functions(text) {
            scanned += 1;
            let root = quantified_root(&body);
            if is_quantified_assertion(&body) && !has_non_vacuity_evidence(&body, root.as_deref()) {
                offenders.push(format!("{}::{name}", path.display()));
            }
        }
    }
    // ⭐ R111⑤／R93：运行期计数器 —— 扫过的函数数必须达标。
    assert!(
        scanned >= 300,
        "R93/R111⑤: the scan must cover every test body, only {scanned} seen"
    );
    assert!(
        offenders.is_empty(),
        "these functions quantify over a collection without any non-vacuity evidence: {offenders:#?}"
    );
}
