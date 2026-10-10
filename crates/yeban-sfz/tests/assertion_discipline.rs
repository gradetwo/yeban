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
//! ① 显式 `x.len() >= N`／`> N`　② 宏隐式相等 `assert_eq!(x.len(), N)`（**两种实参顺序**）
//! ③ 非空 `!x.is_empty()`　⑥ 正对照（同一谓词在**另一个实例**上证明能命中，R112）
//! ⑦ 被遍历的集合本身是**字面量数组／固定范围／全大写常量**（按构造非真空）
//!
//! ⛔ **明确不认**（裁决 **R118**：界必须**界定集合大小**）：
//! - **元素值界**（`assert_eq!(x[0], 5)`）—— 它约束的是元素，⛔ 不约束集合有没有元素；
//! - **运行期计数器**（`assert!(checked >= N)`）—— 它界定的是**动作**，⛔ 不是被遍历集合。
//!   这两类各有一条**已知红**自测（「只有值界 ⇒ 必须报无界」「只有计数器 ⇒ 必须报无界」）。
//!
//! ## 绑定追踪的覆盖与**仍盲**的形态（第二十一批实测）
//!
//! 已追踪：`let ok = <含 .all/.any 的表达式>;`（含**块绑定** `let ok = { … };`）
//! 与**简单绑定链** `let w = v.regions();`（逐跳找界）。
//! - 块绑定：注入 `B4`（无界）⇒ **RED**；`B5`（有界）⇒ **ALL_GREEN**（双向都验过）。
//! - ⭐ **跨函数传出的量词：第二十三批已收窄一层**。测试上下文里，**尾位**（返回值就是量词、
//!   且该量词调用是**最外层调用**）的辅助函数**现在算站点**，必须在**本函数内**有界
//!   （`fn helper(v: &[f32]) -> bool { v.iter().all(..) }` ⇒ **红**；函数内加 `assert!(v.len() >= 1)` ⇒ 绿）。
//!   **仍未覆盖**（每条都写明"为什么、能否收窄"）：
//!   1. **多层传递**（helper A 调 helper B）⇒ 需要**调用图**；语法层只能做**有界内联**（一层）。
//!   2. **非尾位传递**（`let ok = v.iter().all(..); ok`）⇒ 需要**返回路径分析**；本判据只认尾位。
//!   3. **界在调用方**（被调函数不收口、由调用方断言参数）⇒ 需要**过程间契约**；放宽必误伤。
//!
//!   ⚠️ **收窄的代价面（实测）**：把规则放宽到"任何非 assert 量词"会误伤 **5 个产线函数**
//!   （`cc_gates_ok`／`has_data`／`numbered`／`parse_velocity_curve_name`／`parse_set_cc_name`）
//!   与 `tests/support/mod.rs::documented_digest` ⇒ 故必须**测试上下文 ＋ 尾位 ＋ 最外层调用**三条同时成立。
//! - **运行期构造的集合名**（`let name = format!(..); let v = map[&name];`）⇒ 名字到集合的映射不在语法里；
//!   ⛔ 不能收窄；**实际风险方向是假阳性**（对"源容器"的界不认作"别名"的界）。
//!
//! ## 两个**真缺陷**的永久守卫（第二十二批发现，第二十三批做成**配对已知红**）
//! 1. **字符字面量必须掩码**：`split([';', '{', '}'])` 里的 `'{'`／`'}'` 会打乱花括号配对。
//!    配对已知红：`mask_impl(unbalanced, false)` 的花括号计数 ≠ `mask` 的计数。
//! 2. **顶层分号必须深度感知**：`&[f32; SEND_COUNT]` 里的 `;` 在**括号内**，
//!    ⛔ 不能用 `contains(';')` 判"无体声明"。配对已知红：`functions_impl(.., false)` 会把该函数整体丢掉。
//!
//! ⚠️ 另（**R119**）：根绑定必须**带标识符边界** —— `bb.len() >= 2` ⛔ 不得给根 `b` 记界
//! （近名对照已进自测）。
//!
//! ## 已登记的局限（如实，⛔ 不假装能核）
//! - 文本扫描器**无法**核实 ⑥ 的正对照与目标**同类型**（R104 同族）。
//! - **绑变量绕过已收窄**（第二十批）：`let ok = x.iter().all(..); assert!(ok);` 这一形态
//!   **现在会被追踪**（`bound_quantifier_root`），并且 `let w = v.regions();` 这类**简单绑定链**
//!   会沿链**逐跳**寻找界（`resolve_one`）。仍然盲的形态：块绑定（`let ok = { .. };`）、
//!   把量词结果**传出去**（返回 `bool` 的辅助函数）、以及运行期构造的集合名。
//! - 只扫 `src/**`（`tests/**` 由同族判据在各自的库里覆盖）。

use std::fs;

/// 区分**字符字面量**与**生命周期**：`'x'`／`'\\n'`／`'\\''` 是字面量，`'a`（后无引号）是生命周期。
fn is_char_literal(bytes: &[u8], index: usize) -> bool {
    let Some(&next) = bytes.get(index + 1) else {
        return false;
    };
    if next == b'\\' {
        return true;
    }
    if next == b'\'' {
        return false;
    }
    // `'x'`：第三个字节必须是 `'`（多字节字符的 UTF-8 首字节也算）
    let mut i = index + 2;
    while i < bytes.len() && i <= index + 5 {
        if bytes[i] == b'\'' {
            return true;
        }
        i += 1;
    }
    false
}

/// 逐**字节**掩码注释、字符串字面量与**字符字面量**（R113：掩码与原文字节等长 ⇒ 偏移可直接映射）。
fn mask(source: &str) -> String {
    mask_impl(source, true)
}

/// `mask_char_literals = false` ＝ **旧的有缺陷实现**（⛔ 不掩码字符字面量）⇒ 用于**配对已知红**。
fn mask_impl(source: &str, mask_char_literals: bool) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0usize;
    #[derive(PartialEq)]
    enum State {
        Code,
        Line,
        Block,
        Str,
        Char,
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
                } else if mask_char_literals && c == b'\'' && is_char_literal(bytes, i) {
                    // ⚠️ 第十九/二十二批实测：**字符字面量**里的 `{`／`}`／`"` 会破坏花括号配对
                    // （`statements()` 里的 `split([';', '{', '}'])` 就是活例）⇒ 必须与字符串同样掩码。
                    state = State::Char;
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
            State::Char => {
                if c == b'\\' {
                    out[i] = b' ';
                    if i + 1 < bytes.len() && bytes[i + 1] != b'\n' {
                        out[i + 1] = b' ';
                    }
                    i += 2;
                } else if c == b'\'' {
                    state = State::Code;
                    i += 1;
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
fn quantified_root(body: &str, in_test: bool) -> Option<String> {
    // ① 绑定式（`let ok = x.iter().all(..); assert!(ok);`）—— 第十九批登记的盲区已收窄。
    if let Some(root) = bound_quantifier_root(body) {
        return Some(root);
    }
    // ② 尾位量词（跨函数盲区的一层收窄）—— ⚠️ **只在测试上下文**里启用：
    //    生产代码里 `self.gates.iter().all(..)` 是**正常干活**，实测会误伤 5 个产线函数。
    if in_test && let Some(root) = tail_quantifier_root(body) {
        return Some(root);
    }
    // ③ 直接式
    let mut rest = body;
    while let Some(index) = find_token(rest, "assert!(") {
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

/// R133：`find` 的**标识符边界**版本。
///
/// ⛔ `find("assert!(")` 会被 `my_assert!(` 骗到；`find("for ")` 会被 `before ` 骗到。
/// 这里要求 token **左侧**不是标识符字符（token 自带 `!`／`(`／空格 ⇒ 右侧无需再判）。
fn find_token(hay: &str, token: &str) -> Option<usize> {
    let bytes = hay.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(token) {
        let start = from + rel;
        if start == 0 || !is_word(bytes[start - 1]) {
            return Some(start);
        }
        from = start + 1;
    }
    None
}

/// R119：**带标识符边界**的出现判定（`bb.len() >= 2` ⛔ 不得给根 `b` 记界）。
fn mentions_ident(hay: &str, ident: &str) -> bool {
    if ident.is_empty() {
        return false;
    }
    let bytes = hay.as_bytes();
    let ident_bytes = ident.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0usize;
    while let Some(rel) = hay[from..].find(ident) {
        let start = from + rel;
        let end = start + ident_bytes.len();
        let left_ok = start == 0 || !is_word(bytes[start - 1]);
        let right_ok = end >= bytes.len() || !is_word(bytes[end]);
        if left_ok && right_ok {
            return true;
        }
        from = start + 1;
    }
    false
}

/// 简单绑定表：`let <name> = <expr>;`（含类型标注）⇒ `name → expr`。
///
/// 只认**单语句、无块**的绑定；遇到块（`{`）或宏体就放弃该条（⛔ 不猜）。
fn let_bindings(body: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    // ① **块绑定**：`let <name> = { <inner> };`（第二十一批收窄的盲区）。
    //    ⛔ 必须先于"按 `;{}` 切分"的处理 —— 切分会把块内容切出去。
    let bytes = body.as_bytes();
    let mut search = 0usize;
    while let Some(rel) = body[search..].find("let ") {
        let start = search + rel;
        let name_start = start + 4;
        let name: String = body[name_start..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !name.is_empty() {
            let after_name = name_start + name.len();
            if let Some(eq_rel) = body[after_name..].find('=') {
                let eq = after_name + eq_rel;
                let mut cursor = eq + 1;
                while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
                    cursor += 1;
                }
                if bytes.get(cursor) == Some(&b'{') {
                    let mut depth = 0i32;
                    let mut end = cursor;
                    for (index, byte) in bytes.iter().enumerate().skip(cursor) {
                        match byte {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    end = index;
                                    break;
                                }
                            }
                            _ => {}
                        }
                    }
                    if end > cursor {
                        out.push((name.clone(), body[cursor + 1..end].to_string()));
                    }
                }
            }
        }
        search = start + 4;
    }
    // ② 单语句绑定（原路径）
    for stmt in statements(body) {
        let trimmed = stmt.trim_start();
        let Some(rest) = trimmed.strip_prefix("let ") else {
            continue;
        };
        let name: String = rest
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let after_name = &rest[name.len()..];
        let Some(eq) = after_name.find('=') else {
            continue;
        };
        let rhs = after_name[eq + 1..].trim();
        if rhs.starts_with('{') {
            continue;
        }
        out.push((name, rhs.to_string()));
    }
    out
}

/// **一步**解析：`let <name> = <rhs>;` ⇒ `<rhs>` 的根（无绑定或自指 ⇒ `None`）。
fn resolve_one(body: &str, name: &str) -> Option<String> {
    let bindings = let_bindings(body);
    let (_, rhs) = bindings.iter().find(|(n, _)| n == name)?;
    let next: String = rhs
        .trim_start()
        .trim_start_matches('&')
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if next.is_empty() || next == name {
        None
    } else {
        Some(next)
    }
}

/// 绑定式的量词断言：`let ok = <expr 含 .all(/.any(>;` 且本体内有 `assert!(ok`／`assert!(!ok`。
///
/// 这是第十九批登记的盲区（a）：**先把量词结果存进变量再断言**。
/// 现在**追踪这一种简单绑定**（更复杂的形态仍盲，见模块文档）。
fn bound_quantifier_root(body: &str) -> Option<String> {
    for (name, rhs) in let_bindings(body) {
        if !rhs.contains(".all(") && !rhs.contains(".any(") {
            continue;
        }
        let asserted = body.contains(&format!("assert!({name}"))
            || body.contains(&format!("assert!({name},"))
            || body.contains(&format!("assert!(!{name}"));
        if !asserted {
            continue;
        }
        let root: String = rhs
            .trim_start()
            .trim_start_matches('&')
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        if !root.is_empty() {
            return Some(root);
        }
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
    let binds_root = |stmt: &str| match root {
        Some(r) => mentions_ident(stmt, r), // R119：必须**带边界**匹配
        None => true,
    };
    for stmt in statements(body) {
        if !binds_root(stmt) {
            continue;
        }
        // 只认**界定集合大小**的三形态（R118）：
        // ① 显式 `len() >= N`　② 宏隐式相等 `assert_eq!(…len(), N)`（两种实参顺序）　③ `!is_empty()`
        if stmt.contains(".len() >=")
            || stmt.contains(".len() > ")
            || stmt.contains(".count() >=")
            || stmt.contains(".count() > ")
            || (stmt.contains("assert_eq!(") && stmt.contains(".len()"))
            || stmt.contains("!.is_empty()")
            || non_empty_call(stmt)
        {
            return true;
        }
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

/// 有没有 `for <pat> in <字面量/范围/常量>`，且**该迭代表达式的根是 `root`**。
fn iterates_literal(body: &str, root: &str) -> bool {
    let mut rest = body;
    while let Some(index) = find_token(rest, "for ") {
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

/// ⑦ 字面量集合：`for <pat> in <expr>` 且 **`<expr>` 本身**是字面量数组／固定范围／全大写常量。
///
/// ⚠️ 必须**锚定到 `for` 的迭代表达式**，⛔ 不能写成"body 里含 `..`"。
/// **实测（R131：可证伪的推断先量再写）**：本 crate 448 个测试函数里**只有 33%** 的体内含 `..`；
/// 含量词断言的 16 个里，旧宽写法会漏判 **2/16**（`r131_measure.py` 的读数）。
/// 复核命令：`python3 -B .mod-sfz/r131_measure.py`。
///
/// ⚠️ **并更正一条我曾写错的根因**：第十八批我说"旧判据因 `..` 太宽而形同虚设"。
/// 两次**受控实验**（把 `A1`＝删掉一句 `len()` 界注入进去）给出相反结论：
/// - 把 **④ 值界**加回分类器 ⇒ **GREEN（1 passed）** ⇒ **④ 才是当时放行它的原因**；
/// - 去掉 ④、只保留**旧宽 ⑦** ⇒ **RED** ⇒ 旧宽 ⑦ **不是**那次假绿的原因。
///   复现命令：在证据判定里加回 `stmt.contains("vec![")` 那一支（或按 git 历史取 `dd1ed5b~1` 的版本），
///   再施加 `A1` 后跑 `cargo test -p yeban-sfz --test assertion_discipline`。
///   ⇒ R118（只认**界定集合大小**的界）**同时**修掉了这两处：去掉了 ④⑤，也锚定了 ⑦。
fn literal_iteration(body: &str) -> bool {
    let mut rest = body;
    while let Some(index) = find_token(rest, "for ") {
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
    while let Some(index) = find_token(rest, "assert!(") {
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

/// 在 `text` 里找**顶层**（方括号／圆括号深度 0）的 `;`。
///
/// ⚠️ 第二十二批实测：⛔ 不能用 `text.contains(';')` —— 返回类型里的数组长度写作
/// `&[f32; SEND_COUNT]`／`[VoiceHandle; 3]`／`[(Warning, &'static str); 8]`，
/// 那个 `;` **在括号内**。早先的实现据此把这三个**有体**的函数判成"无体声明"⇒
/// 它们的函数体**从未被扫描**（实测未纳管 3 个，逐个核对：`effect::sends`、
/// `parser::warning_display_cases`、`voice_pool::fade_fixture`）。
fn has_top_level_semicolon(text: &str) -> bool {
    let mut depth = 0i32;
    for byte in text.bytes() {
        match byte {
            b'[' | b'(' => depth += 1,
            b']' | b')' => depth -= 1,
            b';' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

/// 从 `(` 开始找到配对的 `)`（返回其**后一位**的下标）。
fn skip_parens(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0i32;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
    }
    None
}

/// 把源码切成 `(函数名, 函数体)`（按花括号配对；**含带参数的函数**）。
///
/// R140：窗口 ＝ **函数体边界**（⛔ 不是固定宽度）；⛔ 只认 `fn name()` 会漏掉**带参数**的函数
/// —— 第二十一批实测：旧规则命中 **448/709**，改成"跳过参数表 ＋ 要求 `{` 先于 `;`"后 **700/710**。
fn functions(source: &str) -> Vec<(String, String, usize)> {
    functions_impl(source, true, true)
}

/// `top_level_semicolon = false` ＝ **旧的有缺陷实现**（`contains(';')`）⇒ 用于**配对已知红**。
fn functions_impl(
    source: &str,
    mask_char_literals: bool,
    top_level_semicolon: bool,
) -> Vec<(String, String, usize)> {
    let masked = mask_impl(source, mask_char_literals);
    let bytes = masked.as_bytes();
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = find_token(&masked[search..], "fn ") {
        let start = search + rel;
        let name_start = start + 3;
        let name_end = masked[name_start..]
            .find('(')
            .map(|e| name_start + e)
            .unwrap_or(name_start);
        let name = masked[name_start..name_end].trim().to_string();
        // R140：⛔ 不能只认 `fn name()`（那会漏掉**全部带参数的函数** —— 实测 448/709）。
        // 这里跳过参数表，再要求 `{` **先于** `;`（后者表示 trait／声明，无函数体）。
        let Some(params_end) = skip_parens(&masked, name_end) else {
            break;
        };
        let Some(brace_rel) = masked[params_end..].find('{') else {
            break;
        };
        let signature = &masked[params_end..params_end + brace_rel];
        let bodyless = if top_level_semicolon {
            has_top_level_semicolon(signature)
        } else {
            signature.contains(';')
        };
        if bodyless {
            search = params_end + 1;
            continue;
        }
        let brace = params_end + brace_rel;
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
            out.push((name, source[brace..end].to_string(), start));
        }
        search = end.max(name_end + 1);
    }
    out
}

/// R131：把**缺界的那一句**摘出来（含量词断言的那条语句）。
fn quantified_snippet(body: &str, root: Option<&str>) -> String {
    for stmt in statements(body) {
        if !stmt.contains(".all(") && !stmt.contains(".any(") {
            continue;
        }
        if root.is_some_and(|r| !mentions_ident(stmt, r)) {
            continue;
        }
        let trimmed = stmt.trim();
        // R137：⛔ 不能按**字节**切多字节源码（会 panic）⇒ 先退到字符边界（`is_char_boundary`）。
        let mut end = trimmed.len().min(160);
        while end > 0 && !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        return if trimmed.len() > end {
            format!("{}…", &trimmed[..end])
        } else {
            trimmed.to_string()
        };
    }
    "<未定位到量词语句 —— 先怀疑检查器>".to_string()
}

/// 本函数体内的量词断言是否**有界**（先按原根，再按 `resolve_root` 回溯的根各试一次）。
fn is_evidenced(body: &str, in_test: bool) -> bool {
    let Some(root) = quantified_root(body, in_test) else {
        return has_non_vacuity_evidence(body, None);
    };
    // 沿绑定链**逐跳**都试一次（⛔ 不能只试链尾：`w → v → f` 里界常写在中途的 `v` 上）。
    let mut current = root.clone();
    for _ in 0..5 {
        if has_non_vacuity_evidence(body, Some(current.as_str())) {
            return true;
        }
        let Some(next) = resolve_one(body, &current) else {
            break;
        };
        current = next;
    }
    has_non_vacuity_evidence(body, Some(root.as_str()))
}

/// **尾位量词**：函数把量词表达式的值**返回**出去（`fn f(v) -> bool { v.iter().all(..) }`）。
///
/// 这是跨函数盲区（`C2`）的**一层收窄**：把"辅助函数体内的量词"也当成**站点**，
/// 从而要求它在本函数内有界。⛔ 只看**尾位**（最后一条语句）—— 否则会误伤
/// `tests/support/mod.rs::documented_digest` 里 `cell.bytes().all(is_ascii_hexdigit)`
/// 这种**正常干活**的量词（实测爆炸半径 1，故必须收紧）。
fn tail_quantifier_root(body: &str) -> Option<String> {
    if !body.contains(".all(") && !body.contains(".any(") {
        return None;
    }
    // 去掉外层花括号
    let inner = body
        .trim()
        .strip_prefix('{')
        .and_then(|rest| rest.strip_suffix('}'))
        .unwrap_or(body);
    // 最后一条**顶层**语句（深度 0 的最后一个 `;` 之后）
    let mut depth = 0i32;
    let mut last = 0usize;
    for (index, ch) in inner.char_indices() {
        match ch {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            ';' if depth == 0 => last = index + 1,
            _ => {}
        }
    }
    let tail = inner[last..].trim();
    if !tail.contains(".all(") && !tail.contains(".any(") {
        return None;
    }
    // ⛔ 必须是**最外层调用**：量词调用的配对右括号必须就在尾部结尾 ——
    //   `text.bytes().all(..) == false`（比较）与 `lines.find(|l| l.bytes().all(..))`
    //   （量词**嵌在别的调用里**，`documented_digest` 就是这一形态）都**不是**"返回值就是量词"。
    let mut outermost_end = None;
    for marker in [".all(", ".any("] {
        if let Some(pos) = tail.find(marker) {
            let mut depth = 0i32;
            for (offset, ch) in tail[pos..].char_indices() {
                match ch {
                    '(' => depth += 1,
                    ')' => {
                        depth -= 1;
                        if depth == 0 {
                            outermost_end = Some(pos + offset + 1);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            break;
        }
    }
    if outermost_end != Some(tail.len()) {
        return None;
    }
    // 接收者链的根
    for marker in [".all(", ".any("] {
        if let Some(pos) = tail.find(marker) {
            // 取接收者链的**第一个**标识符（`v.iter()` ⇒ `v`；⛔ 不是 `iter`）。
            let receiver = tail[..pos].trim_start().trim_start_matches('&');
            let root: String = receiver
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if !root.is_empty() {
                return Some(root);
            }
        }
    }
    None
}

/// 量化断言：`assert!( … .all(` 或 `assert!( ! … .any(`（两类真空面）。
fn is_quantified_assertion(body: &str, in_test: bool) -> bool {
    if bound_quantifier_root(body).is_some() {
        return true;
    }
    if in_test && tail_quantifier_root(body).is_some() {
        return true;
    }
    let mut rest = body;
    while let Some(index) = find_token(rest, "assert!(") {
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
        // ⑥ 正对照
        "let v = f(); assert!(!v.iter().any(|x| *x > 0)); let c = g(); assert!(c.iter().any(|x| *x > 0));",
        // ⑦ 字面量集合
        "for x in [1, 2, 3] { assert!(x > 0); }",
        // 绑定式（盲区收窄）：量词结果先存变量，但**界在同一函数里对同一根给出**
        "let v = f(); let ok = v.iter().all(|x| *x > 0); assert!(ok); assert!(v.len() >= 3);",
        // 绑定式且界指向**绑定的**那个集合（`w`）—— 通过 resolve_root 回溯
        "let v = f(); let w = v.regions(); let ok = w.iter().all(|x| *x > 0); assert!(ok); assert_eq!(v.regions().len(), 3);",
        // `C2` 收窄（第二十三批）：**尾位**量词的辅助函数，且**本函数内有界**
        "{ assert!(v.len() >= 1); v.iter().all(|x| *x > 0.0) }",
        // 块绑定（第二十一批收窄）：`let ok = { <含量词表达式> };` 且**有界**
        "let v = f(); let ok = { v.iter().all(|x| *x > 0) }; assert!(ok); assert!(v.len() >= 3);",
        // ② 的**实参换序**（必须同样被接受）
        "let v = f(); assert_eq!(4, v.len()); assert!(v.iter().all(|x| *x > 0));",
    ];
    let reds = [
        "let v = f(); assert!(v.iter().all(|x| *x > 0));",
        "let v = f(); assert!(!v.iter().any(|x| *x > 0));",
        // ⚠️ R118：**只有值界**（元素值）⇒ 不界定集合大小 ⇒ 必须报无界
        "let v = f(); assert_eq!(v[0], 5); assert!(v.iter().all(|x| *x > 0));",
        // ⚠️ R118：**只有运行期计数器**（界定的是动作，不是被遍历集合）⇒ 必须报无界
        "let v = f(); let mut checked = 0; for x in &v { checked += 1; } assert!(checked >= 3); assert!(v.iter().all(|x| *x > 0));",
        // ⚠️ R119：近名（`bb` 不得给 `b` 记界）
        "let v = f(); let bb = g(); assert!(bb.len() >= 2); assert!(v.iter().all(|x| *x > 0));",
        // ⚠️ 绑定式但**没有界** ⇒ 必须报无界（这是第十九批的盲区，现已收窄）
        "let v = f(); let ok = v.iter().all(|x| *x > 0); assert!(ok);",
        // ⚠️ `C2` 的尾位量词**没有界** ⇒ 必须报无界（跨函数盲区收窄后的红臂）
        "{ v.iter().all(|x| *x > 0.0) }",
        // ⚠️ 块绑定但**没有界** ⇒ 必须报无界
        "let v = f(); let ok = { v.iter().all(|x| *x > 0) }; assert!(ok);",
        // ⚠️ 专打 ⑦ 的过宽：别处有 `..`，但被量化的集合**不是**字面量
        "let v = f(); for i in 0..3 { let _ = i; } assert!(v.iter().all(|x| *x > 0));",
        // ⚠️ 同上：别处有 `for … in [..]`，但被量化的集合不是它
        "let v = f(); let w = [1, 2]; for x in w { let _ = x; } assert!(v.iter().all(|x| *x > 0));",
    ];
    for (index, body) in greens.iter().enumerate() {
        // 绿夹具只要求「被判为非真空」（⑦ 那种是**循环**而不是量词断言，不适用量化检查）。
        assert!(
            is_evidenced(body, true),
            "green {index} must be accepted as non-vacuous: {body}"
        );
    }
    for (index, body) in reds.iter().enumerate() {
        assert!(
            is_quantified_assertion(body, true),
            "red {index} must be quantified"
        );
        assert!(
            !is_evidenced(body, true),
            "red {index} must be rejected: {body}"
        );
    }
    // ⭐ 第二十二批实测：返回类型里的**数组长度分号**会让"无体声明"判定误伤。
    let array_return =
        "fn a() -> [u8; 3] { assert!(v.len() >= 1); assert!(v.iter().all(|x| *x > 0)); }";
    let captured = functions(array_return);
    assert_eq!(
        captured.len(),
        1,
        "an array-typed return must still be captured"
    );
    assert!(
        captured[0].1.contains(".all("),
        "the body of an array-typed function must be scanned: {:?}",
        captured[0].1
    );

    // ⭐ 第二十二批实测：**字符字面量**里的花括号会破坏配对（`['{', '}']` 是活例）。
    // 控制：掩码后 `'{'` 必须消失；且 `functions()` 仍能拿到**完整的**函数体（含量词断言）。
    let char_literal_src = "fn a() { let pairs = ['{', '}']; assert!(v.len() >= 1); assert!(v.iter().all(|x| *x > 0)); }";
    assert!(
        !mask(char_literal_src).contains("'{'"),
        "char literals must be masked like strings"
    );
    let captured = functions(char_literal_src);
    assert_eq!(captured.len(), 1, "exactly one function must be captured");
    assert!(
        captured[0].1.contains(".all("),
        "the whole body must be captured despite char-literal braces: {:?}",
        captured[0].1
    );
    assert!(
        is_evidenced(&captured[0].1, true),
        "the captured body must still be judged as bounded"
    );

    // ⛔ **不得被当成站点**的对照（避免误伤正常干活的量词；实测 `documented_digest` 是这一形态）。
    let non_sites = [
        // 比较形态：量词在 `== false` 里 ⇒ 不是"返回值就是量词"
        "{ text.bytes().all(|b| b.is_ascii_hexdigit()) == false }",
        // 量词在**非尾位**语句里（存进变量但**没有被断言**）⇒ 不是站点
        "{ let ok = text.bytes().all(|b| b.is_ascii_hexdigit()); let _ = ok; 1 }",
    ];
    for (index, body) in non_sites.iter().enumerate() {
        assert!(
            !is_quantified_assertion(body, true),
            "non-site {index} must NOT be treated as a quantified assertion: {body}"
        );
    }

    // ⭐ 配对已知红（R108／第二十三批）：把两个缺陷**故意复现**一次，证明守卫有牙。
    //  ① 不掩码字符字面量 ⇒ 花括号计数必然不同（用**不平衡**的字符字面量才可见）。
    let unbalanced = "fn a() { let open = '{'; assert!(v.iter().all(|x| *x > 0)); }";
    assert_eq!(
        mask(unbalanced).matches('{').count(),
        1,
        "fixed rule: the char literal must be masked"
    );
    assert_eq!(
        mask_impl(unbalanced, false).matches('{').count(),
        2,
        "paired known-red: without char-literal masking the brace count is wrong"
    );
    //  ② 旧的分号判定（`contains(';')`）⇒ 把**有体**函数判成无体声明。
    let array_signature = " -> [u8; 3] ";
    assert!(
        array_signature.contains(';'),
        "paired known-red: the naive rule sees a body-less declaration"
    );
    assert!(
        !has_top_level_semicolon(array_signature),
        "fixed rule: a bracket-nested semicolon does not mean body-less"
    );
    let dropped = functions_impl(array_return, true, false);
    assert!(
        dropped.is_empty(),
        "paired known-red: the naive rule must drop the whole function: {dropped:?}"
    );

    // R133：**近名对照** —— `my_assert!(` ⛔ 不得被当成 `assert!(`；`before ` ⛔ 不得被当成 `for `。
    let decoys = [
        "let v = f(); my_assert!(v.iter().all(|x| *x > 0));",
        "let v = f(); x_assert!(v.iter().all(|x| *x > 0));",
    ];
    for (index, body) in decoys.iter().enumerate() {
        assert!(
            !is_quantified_assertion(body, true),
            "decoy {index} must not be counted as a quantified assertion: {body}"
        );
    }
    assert!(find_token("my_assert!(x)", "assert!(").is_none(), "R133");
    assert_eq!(find_token("before x in y", "for "), None, "R133");
    assert!(find_token("assert!(x)", "assert!(").is_some(), "R133 正例");

    // R134：跨平台红的第一嫌疑是**行尾** —— 归一化后 CRLF 与 LF 的判读必须一致。
    let lf = "let v = f();\nassert!(v.len() >= 2);\nassert!(v.iter().all(|x| *x > 0));\n";
    let crlf = lf.replace('\n', "\r\n");
    assert_eq!(
        is_evidenced(&crlf.replace("\r\n", "\n"), true),
        is_evidenced(lf, true),
        "R134: CRLF must not change the verdict (normalize before scanning)"
    );

    // R113：掩码逐字节等长（含多字节内容）
    let raw = "// 中文注释 assert_ne!(1, 2)\nlet a = \"中文\";\nlet b = 3;\n";
    assert_eq!(mask(raw).len(), raw.len(), "the mask must be byte-wise");
    assert!(
        !mask(raw).contains("assert_ne!(1, 2)"),
        "comment decoy must be masked"
    );
}

/// 对一个源文件求**违例清单**（判据名 + 根 + 缺界的那一句）。
///
/// ⭐ R183：把这段抽成**函数**，于是可以用**坏输入**直接证明它有牙
/// —— ⛔ R180 不许把自检断言写成"集合大小界"，坏输入驱动才是正确形态。
fn offenders_in(path: &std::path::Path, text: &str) -> Vec<String> {
    let mut offenders = Vec::new();
    let cfg_test_at = text.find("#[cfg(test)]");
    let is_tests_file = path.components().any(|c| c.as_os_str() == "tests");
    for (name, body, decl_start) in functions(text) {
        let in_test = is_tests_file || cfg_test_at.is_some_and(|at| decl_start > at);
        let root = quantified_root(&body, in_test);
        if is_quantified_assertion(&body, in_test) && !is_evidenced(&body, in_test) {
            let snippet = quantified_snippet(&body, root.as_deref());
            offenders.push(format!(
                "{}::{name}（根 = {:?}）缺界的那一句： {snippet}",
                path.display(),
                root.as_deref().unwrap_or("<无>")
            ));
        }
    }
    offenders
}

#[test]
fn no_unbounded_all_any_assertion_in_this_crate() {
    self_test_classifier();

    // 覆盖面：`src/**` ＋ `tests/**`（含 `tests/support/`）。
    // ⚠️ 早先只扫 `src/**` ⇒ `tests/**` 里的量词断言不受本判据管辖（已登记为局限）；
    // 现在**纳入**（本文件自身也在其中 —— 自测夹具都是**字符串**，会被掩码，⛔ 不会自伤）。
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dirs = [
        root.join("src"),
        root.join("tests"),
        root.join("tests").join("support"),
    ];
    let mut sources = Vec::new();
    let mut crlf_files = 0usize;
    for dir in &dirs {
        for entry in fs::read_dir(dir).expect("read source dir") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let raw = fs::read_to_string(&path).expect("read source");
                // R134：跨平台红的第一嫌疑是**行尾**（内存 LF vs Windows 检出 CRLF）⇒
                // 归一化后再扫描；把"有 CRLF 的文件数"作为**诊断**打印，⛔ 不作失败条件。
                let text = raw.replace("\r\n", "\n");
                if raw.len() != text.len() {
                    crlf_files += 1;
                }
                sources.push((path, text));
            }
        }
    }
    eprintln!("R134 诊断：含 CRLF 的源文件数 = {crlf_files}（归一化后扫描；⛔ 不是失败条件）");
    // ⭐ R183／R180：自检**不用集合大小界**，改用**喂坏输入**证明扫描管线有牙。
    //   绿臂：有界体 ⇒ 0 条违例；红臂：真空体 ⇒ **恰好 1 条**，且**点名判据与根**。
    let clean_source = "#[test]\nfn t() { let v = f(); assert!(v.len() >= 1); assert!(v.iter().all(|x| *x > 0)); }\n";
    let bad_source = "#[test]\nfn t() { let v = f(); assert!(v.iter().all(|x| *x > 0)); }\n";
    let probe = std::path::Path::new("probe.rs");
    assert!(
        offenders_in(probe, clean_source).is_empty(),
        "green arm: a bounded body must yield no offender"
    );
    let bad_found = offenders_in(probe, bad_source);
    assert_eq!(
        bad_found.len(),
        1,
        "red arm: an unbounded quantifier must yield exactly one offender: {bad_found:?}"
    );
    assert!(
        bad_found[0].contains("t（根 = \"v\"）"),
        "the offender must name the criterion and the root: {bad_found:?}"
    );
    // 规模只作**诊断**打印（⛔ 不是失败条件）。
    eprintln!("R183 诊断：源文件 = {}（仅供阅读）", sources.len());

    let mut offenders = Vec::new();
    let mut scanned = 0usize;
    for (path, text) in &sources {
        let found = offenders_in(path, text);
        scanned += functions(text).len();
        offenders.extend(found);
    }
    // 扫过的函数数只作**诊断**（⛔ R180：不许当自检界）。
    eprintln!("R183 诊断：扫过的函数 = {scanned}（仅供阅读）");
    assert!(
        offenders.is_empty(),
        "these functions quantify over a collection without any non-vacuity evidence: {offenders:#?}"
    );
}
