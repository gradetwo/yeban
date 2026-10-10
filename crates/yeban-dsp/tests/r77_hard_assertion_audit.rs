//! **R77 硬断言全表（常驻判据 ＋ 自带正负对照，R115 范式）**
//!
//! # 为什么需要它
//!
//! 本 crate 有大量"**我们定字节**"的硬断言判据（把 `f32` 位型或字面摘要钉死）。
//! ⚠ **这样的判据只有在被断言的量真的是 IEEE 精确类时才可跨架构比对**：
//! 只要**产线可达集**里出现一个走**宿主 libm** 的超越函数
//! （`sin`／`cos`／`tan`／`exp`／`ln`／`log10`／`log2`／`powf`／`powi`），
//! 这些位型就**只在本架构上有意义**（裁决 R24／R25／R77）。
//!
//! ⚠ 本判据的**存在理由是一次真实事故**：`Wavetable` 的表位型摘要曾被硬断言，
//! 本地（aarch64 ＝ 冻结架构）**必然全绿**，而 CI 在 x86_64 与 Windows 上双双报红
//! （`render_level` 对每个表项算 `phase.sin()`）。⇒ **本地全绿不是证据**。
//!
//! # 本文件做什么
//!
//! 1. 扫 `src/**/*.rs` 的**全部 `#[test]` 判据**；
//! 2. 把含字面量比较的判据分成两类：**浮点位型/摘要**（体内有 `to_bits()`）
//!    与**整数计数**（只比整数/长度）；
//! 3. 为**浮点**类建**产线函数图**（⛔ 排除 `mod tests`）并求可达闭包，
//!    扫其中的宿主 libm 超越函数 ⇒ 分类为 **可硬断言** / **平台感知**；
//! 4. **把结果与提交在仓库里的证据表逐字比对**（表漂了 ⇒ 判据红）；
//! 5. ⭐ **R93／R100 非真空下界**：扫描域（文件数）与每一类的**计数地板**都必须有余量
//!    —— ⛔ 否则"一条都没扫到"会**真空通过**。
//!
//! ⭐ **R115 自带正负对照**：分类器是**对源码文本的纯函数**
//! （[`classify_source`]），本文件用**合成的正例与负例**喂它
//! （[`the_classifier_separates_float_bits_from_integer_counts`]），
//! 因此它**不需要**往产线代码里做破坏性注入即可被喂红。
//!
//! ⭐ **R111③ 幂等**：设环境变量 `R77_WRITE=1` 时本判据会**重写**证据表
//! （⛔ 仅在 `cargo fmt` 之后运行本判据才允许，因为格式化会移动行号/换行）。

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

/// 被审计的源码根（相对 crate 根）。
const SRC_ROOT: &str = "src";

/// 提交在仓库里的证据表（相对 crate 根）。
const TABLE_PATH: &str = "tests/data/hard_assertion_table.txt";

/// ⭐ **棘轮入口表**（R132）：`platform` 类契约里**尚未在源码中标注平台依赖**的那些。
///
/// 表里的每一行都是一个**未修缺口**（口径：该判据用位型比较，却没有在源码里声明
/// "我只在冻结架构上有意义"）。本判据把它当作**棘轮**：
/// 入口**只能减少** —— 新增未标注的 `platform` 判据会让判据红，
/// 而给某条判据加上标记后必须**同时**从表里删掉（否则"陈旧入口"也会让判据红）。
const ALLOWLIST_PATH: &str = "tests/data/platform_dependent_allowlist.txt";

/// 源码里声明"本判据的平台依赖"的标记（任一出现即算已标注）。
///
/// 依据：本 crate 的既有约定是**位型只在冻结架构（aarch64）内比对**（裁决 R24／R25），
/// 因此"已标注"的形态就是在这几处之一写明这件事。
const PLATFORM_MARKERS: &[&str] = &[
    "冻结架构",
    "平台感知",
    "platform-dependent",
    "FROZEN_ARCHITECTURE",
    "点名跳过",
];

/// 判据里被钉死的量属于哪一类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// 浮点位型或字面摘要（`to_bits()`／64 位字面量）。
    FloatBits,
    /// 整数计数（帧数／长度／命中数…）。
    IntegerCount,
}

impl Kind {
    fn as_str(self) -> &'static str {
        match self {
            Kind::FloatBits => "float_bits",
            Kind::IntegerCount => "integer_count",
        }
    }
}

/// 浮点类契约的可移植性分类。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Class {
    /// 可达产线函数里**没有**宿主 libm 的超越函数 ⇒ 可跨架构硬断言。
    HardAssertable,
    /// 可达产线函数里有宿主 libm 的超越函数 ⇒ **只在冻结架构上有意义**。
    PlatformDependent,
    /// 不调用本 crate 的产线函数（纯夹具／常量比较）。
    NoProductionCallee,
}

impl Class {
    fn as_str(self) -> &'static str {
        match self {
            Class::HardAssertable => "hard",
            Class::PlatformDependent => "platform",
            Class::NoProductionCallee => "no_callee",
        }
    }
}

/// 证据表的一行。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Row {
    file: String,
    test: String,
    kind: Kind,
    reach: usize,
    class: Class,
    host: Vec<String>,
}

/// 走宿主 libm 的超越函数名（[`Class::PlatformDependent`] 的判据）。
const HOST_TRANSCENDENTALS: &[&str] = &[
    ".sin(", ".cos(", ".tan(", ".exp(", ".ln(", ".log10(", ".log2(", ".powf(", ".powi(",
];

/// 纯 Rust（钉死的 `libm` crate）的调用前缀 —— 它**不是**宿主 libm。
const PINNED_LIBM_PREFIX: &str = "libm::";

/// 一个函数的名字与字节区间。
#[derive(Debug, Clone)]
struct FnSpan {
    name: String,
    start: usize,
    end: usize,
}

/// 用大括号配平切出源码里的全部 `fn`（名字 ＋ 体区间）。
fn fn_spans(src: &str) -> Vec<FnSpan> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(offset) = src[cursor..].find("fn ") {
        let start = cursor + offset;
        let after = &src[start + 3..];
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        let brace = match src[start..].find('{') {
            Some(b) => start + b,
            None => break,
        };
        let mut depth = 0i32;
        let mut end = src.len();
        for (k, c) in src[brace..].char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = brace + k;
                        break;
                    }
                }
                _ => {}
            }
        }
        if !name.is_empty() {
            out.push(FnSpan { name, start, end });
        }
        cursor = end + 1;
        if cursor >= src.len() {
            break;
        }
    }
    out
}

/// `mod tests { … }` 区间的起止（用于把测试代码排除在**产线**图之外）。
fn test_module_span(src: &str) -> Option<(usize, usize)> {
    let start = src.find("mod tests")?;
    let brace = src[start..].find('{')? + start;
    let mut depth = 0i32;
    for (k, c) in src[brace..].char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some((brace, brace + k));
                }
            }
            _ => {}
        }
    }
    Some((brace, src.len()))
}

fn is_test_fn(src: &str, span: &FnSpan) -> bool {
    let head = &src[..span.start];
    let tail: String = head
        .chars()
        .rev()
        .take(240)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    tail.contains("#[test]")
}

/// ⭐ **R134 共享助手：读盘 ＋ 行尾归一化 ＋ 报告原始行尾**。
///
/// 凡读 `tests/data/*.txt` 的判据都应走这里：本判据在内存里生成的是 LF，
/// 而 **Windows 检出会把仓库文本变成 CRLF** ⇒ 不归一化就会**只因为行尾**在
/// `windows` 腿上报红（实测：提交 `fd54598` 正是这样红的）。
/// ⛔ **行尾本身不是失败条件**：这里只把原始形态回报给调用者作**诊断**。
fn read_evidence_text(path: &Path) -> (String, bool) {
    let raw = fs::read_to_string(path).unwrap_or_else(|e| panic!("读不到证据文件 {path:?}：{e}"));
    let had_crlf = raw.contains("\r\n");
    (raw.replace("\r\n", "\n"), had_crlf)
}

/// 判据（含**紧邻其上的连续上下文**）里是否声明了平台依赖。
///
/// ⭐ **R140**：窗口**不是固定宽度**，而是**函数体边界** ＋ 向上的**连续上下文**
/// （`///`／`//!`／`#[…]`／`//`／空行都算，遇到别的代码即停）。
/// 固定宽度的两种错法：**太窄 ⇒ 漏**（假阳）；**太宽 ⇒ 借到邻居的标记**（假阴，R114 明禁）。
///
/// ⭐ **R133 near-miss**：函数名用 [`fn_spans`] 的**精确相等**匹配，
/// ⛔ 不是 `contains("fn {test}(")` —— 后者会把 `fn {test}_x(` 借给 `{test}`。
fn declares_platform_dependence(file_src: &str, test: &str) -> bool {
    let Some(span) = fn_spans(file_src).into_iter().find(|s| s.name == test) else {
        return false;
    };
    // ⚠ 先定位 `fn` **自己所在的行**，再从它的**上一行**开始向上走连续上下文
    // （第一版直接从 `span.start` 往上找换行 ⇒ 拿到的是 `fn` 那行本身 ⇒ 立刻 break ⇒ 永远漏标记）。
    let lines: Vec<&str> = file_src.split('\n').collect();
    let mut offset = 0usize;
    let mut fn_line = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if offset <= span.start && span.start <= offset + line.len() {
            fn_line = index;
            break;
        }
        offset += line.len() + 1;
    }
    let mut start_line = fn_line;
    while start_line > 0 {
        let prev = lines.get(start_line - 1).copied().unwrap_or("");
        let trimmed = prev.trim_start();
        let is_context = trimmed.is_empty()
            || trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with("#[")
            || trimmed.starts_with("//");
        if !is_context {
            break;
        }
        start_line -= 1;
    }
    let mut start = 0usize;
    for line in lines.iter().take(start_line) {
        start += line.len() + 1;
    }
    let region = &file_src[start..span.end];
    PLATFORM_MARKERS.iter().any(|m| region.contains(m))
}

/// **函数体里是否调用了 `name`**。
///
/// ⭐ **R133 near-miss**：`my_bar(` ⛔ 不算调用 `bar` —— 匹配处**前一个字符**不得是
/// 标识符字符（字母/数字/下划线）。这正是 `contains(&format!("{name}("))` 的孪生点缺陷。
fn calls_function(body: &str, name: &str) -> bool {
    let needle = format!("{name}(");
    let mut cursor = 0usize;
    while let Some(p) = body[cursor..].find(&needle) {
        let at = cursor + p;
        let boundary_ok = match body[..at].chars().next_back() {
            None => true,
            Some(prev) => !(prev.is_alphanumeric() || prev == '_'),
        };
        if boundary_ok {
            return true;
        }
        cursor = at + needle.len();
        if cursor >= body.len() {
            break;
        }
    }
    false
}

/// 源码里出现的宿主超越函数名（去重、排序）。
fn host_transcendentals(body: &str) -> Vec<String> {
    let mut set = BTreeSet::new();
    for name in HOST_TRANSCENDENTALS {
        if body.contains(name) {
            set.insert(
                (*name)
                    .trim_start_matches('.')
                    .trim_end_matches('(')
                    .to_string(),
            );
        }
    }
    set.into_iter().collect()
}

/// 一段 `assert` 实参里是否有**整数**字面量（⛔ 排除浮点与十六进制位型）。
fn has_integer_literal(seg: &str) -> bool {
    let bytes = seg.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            // 跳过 `0x…` 十六进制字面量（它们是位型，不是计数）。
            if i > 0 && (bytes[i - 1] == b'x' || bytes[i - 1] == b'X') {
                i += 1;
                continue;
            }
            let start = i;
            while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                i += 1;
            }
            let is_float = i < bytes.len() && bytes[i] == b'.';
            let type_suffix =
                i < bytes.len() && (bytes[i] == b'f' || bytes[i] == b'i' || bytes[i] == b'u');
            if !is_float && !type_suffix && i > start {
                return true;
            }
        } else {
            i += 1;
        }
    }
    false
}

/// 一段 `assert` 实参里是否有 ≥6 位的十六进制字面量（位型/摘要）。
fn has_long_hex(seg: &str) -> bool {
    let mut cursor = 0usize;
    while let Some(p) = seg[cursor..].find("0x") {
        let start = cursor + p + 2;
        let digits = seg[start..]
            .chars()
            .take_while(|c| c.is_ascii_hexdigit() || *c == '_')
            .count();
        if digits >= 6 {
            return true;
        }
        cursor = start;
        if cursor >= seg.len() {
            break;
        }
    }
    false
}

/// **判据体的断言分类**：`(是否是浮点位型/摘要, 是否是整数计数)`。
///
/// ⚠ 按**逐条 assert** 判定，⛔ 不是"体内出现过 `to_bits()` 就算浮点" ——
/// 一个判据可以同时含两类断言（那时**两类都记**）。
fn assert_kinds(body: &str) -> (bool, bool) {
    let mut float = false;
    let mut int = false;
    let mut cursor = 0usize;
    while let Some(p) = body[cursor..].find("assert") {
        let start = cursor + p;
        let end = body[start..]
            .find(");")
            .map_or_else(|| (start + 240).min(body.len()), |e| start + e);
        let seg = &body[start..end];
        if seg.contains("to_bits()") || has_long_hex(seg) {
            float = true;
        } else if has_integer_literal(seg) {
            int = true;
        }
        cursor = end + 1;
        if cursor >= body.len() {
            break;
        }
    }
    (float, int)
}

/// **分类器（对源码文本的纯函数）**：返回该文件里全部硬断言判据的分类行。
fn classify_source(file: &str, src: &str) -> Vec<Row> {
    let spans = fn_spans(src);
    let tests_span = test_module_span(src);
    let in_tests = |offset: usize| tests_span.is_some_and(|(a, b)| offset >= a && offset <= b);

    // 产线函数（在 `mod tests` 之外）。
    let mut prod: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for span in &spans {
        if !in_tests(span.start) {
            prod.entry(span.name.clone())
                .or_insert((span.start, span.end));
        }
    }
    let prod_names: BTreeSet<&String> = prod.keys().collect();

    // 调用边（只连产线函数）。
    let mut edges: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (name, (a, b)) in &prod {
        let body = &src[*a..*b];
        let mut set = BTreeSet::new();
        for other in &prod_names {
            if *other == name {
                continue;
            }
            // ⭐ R133：用带标识符边界的 `calls_function`，⛔ 不用裸 `contains("{name}(")`。
            if calls_function(body, other) {
                set.insert((*other).clone());
            }
        }
        edges.insert(name.clone(), set);
    }

    let closure = |start: &str| -> BTreeSet<String> {
        let mut seen = BTreeSet::new();
        let mut stack = vec![start.to_string()];
        while let Some(n) = stack.pop() {
            if let Some(next) = edges.get(&n) {
                for c in next {
                    if seen.insert(c.clone()) {
                        stack.push(c.clone());
                    }
                }
            }
        }
        seen
    };

    let mut rows = Vec::new();
    for span in &spans {
        if !is_test_fn(src, span) {
            continue;
        }
        let body = &src[span.start..span.end];
        let (float_bits, int_count) = assert_kinds(body);
        if !float_bits && !int_count {
            continue;
        }
        if !float_bits {
            rows.push(Row {
                file: file.to_string(),
                test: span.name.clone(),
                kind: Kind::IntegerCount,
                reach: 0,
                class: Class::NoProductionCallee,
                host: Vec::new(),
            });
            continue;
        }
        // 浮点类：求产线可达闭包（测试函数本身不在产线图里，故先取它调用的产线函数）。
        let mut reached = BTreeSet::new();
        let mut direct = BTreeSet::new();
        for other in &prod_names {
            if calls_function(body, other) {
                direct.insert((*other).clone());
            }
        }
        for d in &direct {
            reached.insert(d.clone());
            reached.extend(closure(d));
        }
        let mut host = BTreeSet::new();
        let mut pinned = false;
        for name in &reached {
            if let Some((a, b)) = prod.get(name) {
                let fbody = &src[*a..*b];
                for t in host_transcendentals(fbody) {
                    host.insert(t);
                }
                if fbody.contains(PINNED_LIBM_PREFIX) {
                    pinned = true;
                }
            }
        }
        let class = if reached.is_empty() {
            Class::NoProductionCallee
        } else if !host.is_empty() {
            Class::PlatformDependent
        } else {
            let _ = pinned;
            Class::HardAssertable
        };
        rows.push(Row {
            file: file.to_string(),
            test: span.name.clone(),
            kind: Kind::FloatBits,
            reach: reached.len(),
            class,
            host: host.into_iter().collect(),
        });
    }
    rows.sort();
    rows
}

/// 递归收集源码文件（`*.rs`，排除 `mod tests` 之外的目录差异）。
fn source_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// 审计整棵树。
fn audit_tree(crate_root: &Path) -> Vec<Row> {
    let mut rows = Vec::new();
    for path in source_files(&crate_root.join(SRC_ROOT)) {
        let rel = path
            .strip_prefix(crate_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let src = fs::read_to_string(&path).unwrap_or_default();
        rows.extend(classify_source(&rel, &src));
    }
    rows.sort();
    rows
}

/// 把分类结果渲染成证据表（确定性文本）。
fn render_table(rows: &[Row]) -> String {
    let floats = rows.iter().filter(|r| r.kind == Kind::FloatBits).count();
    let ints = rows.iter().filter(|r| r.kind == Kind::IntegerCount).count();
    let platform = rows
        .iter()
        .filter(|r| r.class == Class::PlatformDependent)
        .count();
    let hard = rows
        .iter()
        .filter(|r| r.class == Class::HardAssertable)
        .count();
    let mut out = String::new();
    out.push_str("# R77 硬断言全表（由 tests/r77_hard_assertion_audit.rs 生成并逐字校验）\n");
    out.push_str(
        "# 分类口径：float_bits ＝ 判据体内有 to_bits()；integer_count ＝ 只比整数/长度\n",
    );
    out.push_str(
        "# class：hard ＝ 产线可达集无宿主 libm 超越函数；platform ＝ 有（只在本架构有意义）\n",
    );
    out.push_str("#        no_callee ＝ 不调用本 crate 产线函数\n");
    out.push_str(&format!(
        "# 合计 {} 条：float_bits {} ／ integer_count {}；其中 hard {} ／ platform {}\n",
        rows.len(),
        floats,
        ints,
        hard,
        platform
    ));
    out.push_str("# 列：kind | class | reach | host | file | test\n");
    for r in rows {
        out.push_str(&format!(
            "{} | {} | {} | {} | {} | {}\n",
            r.kind.as_str(),
            r.class.as_str(),
            r.reach,
            if r.host.is_empty() {
                "-".to_string()
            } else {
                r.host.join(",")
            },
            r.file,
            r.test
        ));
    }
    out
}

// ---------------------------------------------------------------------------
// ⭐ R115 正负对照：分类器是纯函数，用**合成源码**喂它
// ---------------------------------------------------------------------------

/// **已知红／已知绿对照**：同一个分类器，喂合成源码。
///
/// - **正例（已知红）**：一条 `to_bits()` 判据，其产线路径里有一个 `phase.sin()`
///   ⇒ 必须被判为 `float_bits` ＋ `platform`；
/// - **负例（已知绿）**：一条只比帧数的判据 ⇒ 必须被判为 `integer_count`；
/// - **对照（可硬断言）**：一条 `to_bits()` 判据，产线路径只有乘加
///   ⇒ 必须被判为 `float_bits` ＋ `hard`。
///
/// ⭐ 没有这三个对照，[`Class::PlatformDependent`] 与 [`Kind::IntegerCount`]
/// 就可能**永远返回**同一个值而判据**真空通过**（R93）。
#[test]
fn the_classifier_separates_float_bits_from_integer_counts() {
    let synthetic = r#"
fn render_level(len: usize) -> f32 {
    let phase = 1.0f32;
    phase.sin()
}
fn pure_add(a: f32, b: f32) -> f32 {
    a + b
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_float_criterion_with_a_transcendental() {
        assert_eq!(render_level(4).to_bits(), 0x3f80_0000);
    }
    #[test]
    fn a_pure_float_criterion() {
        assert_eq!(pure_add(1.0, 2.0).to_bits(), 0x4040_0000);
    }
    #[test]
    fn an_integer_criterion() {
        let frames = 4usize;
        assert!(frames >= 4);
    }
}
"#;
    let rows = classify_source("synthetic.rs", synthetic);
    let find = |name: &str| rows.iter().find(|r| r.test == name).cloned();

    let red = find("a_float_criterion_with_a_transcendental").expect("正例必须被扫到");
    assert_eq!(red.kind, Kind::FloatBits, "正例必须是浮点类");
    assert_eq!(
        red.class,
        Class::PlatformDependent,
        "正例的可达集含 `.sin(` ⇒ 必须判平台感知（否则分类器对宿主 libm 无判别力）"
    );
    assert_eq!(red.host, vec!["sin".to_string()], "必须点名 `sin`");

    let green = find("a_pure_float_criterion").expect("对照必须被扫到");
    assert_eq!(green.kind, Kind::FloatBits);
    assert_eq!(
        green.class,
        Class::HardAssertable,
        "可达集只有 `a + b` ⇒ 必须判可硬断言"
    );

    let ints = find("an_integer_criterion").expect("整数判据必须被扫到");
    assert_eq!(ints.kind, Kind::IntegerCount, "只比长度 ⇒ 必须是整数计数类");

    assert_eq!(rows.len(), 3, "合成源码里恰好 3 条判据（⛔ 不多不少）");
}

// ---------------------------------------------------------------------------
// 常驻判据：证据表逐字校验 ＋ 非真空下界
// ---------------------------------------------------------------------------

/// **证据表必须与当前源码逐字一致**，且扫描域与每一类计数都有**下界余量**。
#[test]
fn the_hard_assertion_table_matches_the_committed_evidence() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rows = audit_tree(&crate_root);
    let rendered = render_table(&rows);

    // ⭐ R93／R100 非真空下界：扫描域与每一类都必须有余量。
    let files = source_files(&crate_root.join(SRC_ROOT)).len();
    let floats = rows.iter().filter(|r| r.kind == Kind::FloatBits).count();
    let ints = rows.iter().filter(|r| r.kind == Kind::IntegerCount).count();
    let platform = rows
        .iter()
        .filter(|r| r.class == Class::PlatformDependent)
        .count();
    let hard = rows
        .iter()
        .filter(|r| r.class == Class::HardAssertable)
        .count();
    assert!(
        files >= 20,
        "扫描域太小：只读到 {files} 个源文件（地板 20；实测 25）"
    );
    assert!(
        floats >= 50,
        "浮点类判据太少：只扫到 {floats} 条（地板 50；实测 69）"
    );
    assert!(
        ints >= 200,
        "整数计数判据太少：只扫到 {ints} 条（地板 200；实测 265）"
    );
    assert!(
        platform >= 20,
        "平台感知类太少：只扫到 {platform} 条（地板 20；实测 32）—— 分类器可能退化了"
    );
    assert!(
        hard >= 20,
        "可硬断言类太少：只扫到 {hard} 条（地板 20；实测 34）⇒ 分类器可能退化"
    );

    let table_path = crate_root.join(TABLE_PATH);
    if std::env::var("R77_WRITE").is_ok() {
        fs::write(&table_path, &rendered).expect("写证据表");
        // ⭐ R111③：写完立刻回读，确认落盘内容与刚渲染的一致（幂等）。
        let back = fs::read_to_string(&table_path).expect("回读证据表");
        assert_eq!(back, rendered, "证据表写入后回读不一致");
        return;
    }
    let (committed, had_crlf) = read_evidence_text(&table_path);
    if committed != rendered {
        let committed_lines: Vec<&str> = committed.lines().collect();
        let rendered_lines: Vec<&str> = rendered.lines().collect();
        let first = committed_lines
            .iter()
            .zip(rendered_lines.iter())
            .position(|(a, b)| a != b);
        let detail = first.map_or_else(
            || {
                format!(
                    "前 {} 行相同，长度不同（表 {} 行 / 渲染 {} 行）",
                    committed_lines.len().min(rendered_lines.len()),
                    committed_lines.len(),
                    rendered_lines.len()
                )
            },
            |i| {
                format!(
                    "首个差异行 {}：表={:?} 渲染={:?}",
                    i + 1,
                    committed_lines.get(i).unwrap_or(&"<缺失>"),
                    rendered_lines.get(i).unwrap_or(&"<缺失>")
                )
            },
        );
        panic!("证据表与当前源码不一致（{detail}）：先跑 cargo fmt，再用 R77_WRITE=1 重生成");
    }
    // ⭐ 报告原始行尾（诊断用；⛔ 不作为失败条件）。
    if had_crlf {
        eprintln!("[r77] 注意：证据表在盘上是 CRLF（已在比较前归一化）");
    }
}

// ---------------------------------------------------------------------------
// ⭐ 棘轮判据（R132）：platform 类契约必须声明平台依赖，否则进"入口表"
// ---------------------------------------------------------------------------

/// ⭐ **R132 棘轮**：`platform` 类契约**要么在源码里声明平台依赖**（`冻结架构`／
/// `平台感知`／`platform-dependent`／`FROZEN_ARCHITECTURE`／`点名跳过`），
/// **要么出现在入口表**（＝一个**未修缺口**）。两侧**双向相等**：
/// 新增未标注的 `platform` 判据 ⇒ 红（新缺口）；已标注却仍留在表里 ⇒ 红（陈旧入口）。
///
/// ⛔ **本判据不做的事**：它不把位型比较改成容差 —— 本 crate 的既有约定是
/// "位型只在冻结架构（aarch64）内比对"（裁决 R24／R25），因此**保留位型比较 ＋
/// 就地标注**才是正解；容差只用于**跨架构**的数值断言（本 crate 目前没有）。
#[test]
fn every_platform_dependent_criterion_declares_its_platform_dependence() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rows = audit_tree(&crate_root);
    let platform: Vec<&Row> = rows
        .iter()
        .filter(|r| r.class == Class::PlatformDependent)
        .collect();

    let mut cache: BTreeMap<String, String> = BTreeMap::new();
    let mut unmarked: Vec<String> = Vec::new();
    let mut marked = 0usize;
    for r in &platform {
        let src = cache
            .entry(r.file.clone())
            .or_insert_with(|| fs::read_to_string(crate_root.join(&r.file)).unwrap_or_default());
        if declares_platform_dependence(src, &r.test) {
            marked += 1;
        } else {
            unmarked.push(format!("{}|{}", r.file, r.test));
        }
    }
    unmarked.sort();

    // ⭐ R93 地板：两侧都要有余量，⛔ 否则"一条都没扫到"会真空通过。
    assert!(
        platform.len() >= 20,
        "platform 类太少：{} 条（地板 20）⇒ 分类器可能退化",
        platform.len()
    );
    assert!(
        marked >= 1,
        "没有任何判据被识别为『已标注』⇒ 标记扫描可能失效"
    );

    let allowlist_path = crate_root.join(ALLOWLIST_PATH);
    if std::env::var("R77_WRITE").is_ok() {
        let mut out = String::new();
        out.push_str("# R132 棘轮入口表：platform 类契约里**未在源码标注平台依赖**的那些。\n");
        out.push_str(
            "# 口径见 crates/yeban-dsp/tests/r77_hard_assertion_audit.rs 的 PLATFORM_MARKERS。\n",
        );
        out.push_str(
            "# 本表只能收缩：新增未标注的 platform 判据会让判据红；标注后必须同时删行。\n",
        );
        out.push_str(&format!("# 当前入口数 = {}\n", unmarked.len()));
        for entry in &unmarked {
            out.push_str(&format!("{entry}\n"));
        }
        fs::write(&allowlist_path, &out).expect("写入口表");
        let (back, _) = read_evidence_text(&allowlist_path);
        assert_eq!(back, out, "入口表写入后回读不一致");
        return;
    }

    let (declared, had_crlf) = read_evidence_text(&allowlist_path);
    let entries: Vec<String> = declared
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect();
    assert_eq!(
        entries, unmarked,
        "入口表与实测不一致（新缺口 ＝ 出现未标注的 platform 判据；或陈旧入口 ＝ 已标注却未删行）\
         ⇒ 用 R77_WRITE=1 重生成（⛔ 仅在 cargo fmt 之后）"
    );
    if had_crlf {
        eprintln!("[r77] 注意：入口表在盘上是 CRLF（已在比较前归一化）");
    }
    eprintln!(
        "[r77] R132 进度读数：platform 契约 {} 条｜已标注 {} 条｜**未修入口 {} 条**",
        platform.len(),
        marked,
        entries.len()
    );
}

/// ⭐ **R133 孪生点对照**：两处**标识符匹配**都配 near-miss 对照。
///
/// 本会话已因孪生点踩坑 5 次（函数体 vs 条件路径／宏名／`find_token`／子串绑定…），
/// 因此**凡 `find`／`contains`／`ends_with` 匹配标识符处**都必须有对照：
/// - ① [`declares_platform_dependence`] 用 [`fn_spans`] 的**精确相等**匹配函数名
///   ⇒ `fn foo_x()` 上的标记 ⛔ **不得**借给 `fn foo()`；
/// - ② [`calls_function`] 带**标识符边界**
///   ⇒ `my_bar(` ⛔ **不得**算作调用 `bar`（这正是裸 `contains("bar(")` 的孪生点缺陷）。
#[test]
fn identifier_matching_has_near_miss_controls() {
    // ① 函数名匹配：精确相等。
    let neighbour = "mod tests {\n    /// 冻结架构\n    #[test]\n    fn foo_x() {}\n    #[test]\n    fn foo() { assert_eq!(1u32, 1); }\n}\n";
    assert!(
        !declares_platform_dependence(neighbour, "foo"),
        "`fn foo_x()` 的标记 ⛔ 不得借给 `fn foo()`（孪生点：前缀匹配）"
    );
    let self_marked = "mod tests {\n    /// 冻结架构\n    #[test]\n    fn foo() { assert_eq!(1u32, 1); }\n    #[test]\n    fn foo_x() {}\n}\n";
    assert!(
        declares_platform_dependence(self_marked, "foo"),
        "紧邻 `fn foo()` 上方的标记必须算作它已标注"
    );

    // ② 调用匹配：标识符边界。
    assert!(
        !calls_function("fn user() { my_bar() }", "bar"),
        "`my_bar(` ⛔ 不得算作调用 `bar`（孪生点：后缀/子串匹配）"
    );
    assert!(
        !calls_function("fn user() { bar_x() }", "bar"),
        "`bar_x(` ⛔ 不得算作调用 `bar`"
    );
    assert!(
        calls_function("fn user() { bar() }", "bar"),
        "`bar(` 必须算作调用 `bar`（否则对照自身真空）"
    );
    assert!(
        calls_function("fn user() { self.bar() }", "bar"),
        "`self.bar(` 必须算作调用 `bar`（`.` 不是标识符字符）"
    );

    // ③ 端到端：**受控实验**（R91）——同一判据形态，只改一处调用名。
    // 可达集的定义是 {直接调用的产线函数} ∪ {它们的传递闭包}，所以
    // 调用 `user()`（它又调用 `my_bar()`）时正确计数是 **2**；若把 `my_bar(` 误当成
    // `bar(`（孪生点缺陷），`bar` 会额外进来 ⇒ **3**。
    let near_miss = "fn bar() -> f32 { 1.0 }\nfn my_bar() -> f32 { 2.0 }\nfn user() -> f32 { my_bar() }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert_eq!(user().to_bits(), 0x3f80_0000); }\n}\n";
    let rows = classify_source("synthetic2.rs", near_miss);
    let row = rows.iter().find(|r| r.test == "t").expect("必须扫到 t");
    assert_eq!(
        row.reach, 2,
        "调用 `my_bar()` 的正确可达计数是 2（`user` ＋ `my_bar`）——          孪生点缺陷会额外把 `bar` 算进来 ⇒ 3（实得 {}）",
        row.reach
    );
    assert_eq!(
        row.class,
        Class::HardAssertable,
        "这三个产线函数都无超越函数 ⇒ 必须判可硬断言（顺带确认分类没被计数带偏）"
    );

    // 正对照：真的调用 `bar()` 时计数必须是 3（⇒ 上一条的"2"不是因为少算了东西）。
    let real_call = near_miss.replace(
        "fn user() -> f32 { my_bar() }",
        "fn user() -> f32 { bar() + my_bar() }",
    );
    let rows2 = classify_source("synthetic3.rs", &real_call);
    let row2 = rows2.iter().find(|r| r.test == "t").expect("必须扫到 t");
    assert_eq!(
        row2.reach, 3,
        "真调用 `bar()` 与 `my_bar()` 时可达计数必须是 3（实得 {}）——          否则『2』只是因为少算了",
        row2.reach
    );
}

// ---------------------------------------------------------------------------
// R149／R150：`src/` 内"匹配站点"普查（表 ＋ 判据逐字校验）
// ---------------------------------------------------------------------------

/// 提交在仓库里的匹配站点表。
const MATCH_SITES_PATH: &str = "tests/data/src_match_sites.txt";

/// 被普查的匹配方法名（子串/前后缀匹配都算）。
const MATCH_METHODS: &[&str] = &[
    ".contains(",
    ".find(",
    ".starts_with(",
    ".ends_with(",
    ".strip_prefix(",
    ".strip_suffix(",
];

/// 一行的扫描结果：`(needle, 是否字面量)`。
///
/// ⚠ **R150 自审（口径差异，逐条记录）**：
/// - 本函数是**行式**扫描（⛔ 不跨行）——实测本 crate 里"调用被折行"的站点 **0** 处
///   （`grep -c '\.contains($\|\.find($…'` = 0），因此当前不漏；但这条**是口径差异**，
///   若将来出现折行形态，本函数会漏、而基于 span 的判据不会；
/// - 字面量判定只认**双引号**开头（⛔ 不认原始字符串 `r"…"`／字符字面量 `'x'`）——
///   实测本 crate 的 24 处非双引号 needle 全部是**变量**，无原始字符串/字符字面量；
/// - 注释行与行尾 `\r` 不参与判定（`trim_end` 后匹配）。
fn match_sites_on_line(line: &str) -> Vec<(String, bool, &'static str)> {
    let mut out = Vec::new();
    for method in MATCH_METHODS {
        let bare = method.trim_start_matches('.').trim_end_matches('(');
        let mut cursor = 0usize;
        while let Some(p) = line[cursor..].find(method) {
            let at = cursor + p + method.len();
            if at < line.len() {
                let literal = line[at..].starts_with('"');
                let rest = &line[at..];
                let needle: String = if literal {
                    rest[1..].chars().take_while(|c| *c != '"').collect()
                } else {
                    rest.chars()
                        .take_while(|c| *c != ')' && *c != ',')
                        .collect()
                };
                out.push((needle.trim().to_string(), literal, bare));
            }
            cursor = at;
            if cursor >= line.len() {
                break;
            }
        }
    }
    out
}

/// 该行是否位于某个 `#[test]` 判据体内（向上找最多 45 行）。
fn in_test_context(text: &str, line_index: usize) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let start = line_index.saturating_sub(45);
    lines[start..line_index]
        .iter()
        .any(|l| l.trim_start().starts_with("#[test]"))
}

/// 审计整棵 `src/` 树的匹配站点，返回证据表的文本（6 列：含**判定**与**理由**）。
///
/// **判定规则（机械可导）**：
/// - `starts_with`／`ends_with`／`strip_prefix`／`strip_suffix` 的字面量站
///   ⇒ `by_design`（**前后缀分类器**：按设计，⛔ 不冒充等值匹配）；
/// - `contains`／`find` 的字面量站 **在 `#[test]` 内** ⇒ `by_design`（Debug 形状断言）；
/// - `contains`／`find` 的字面量站 **不在测试内** ⇒ `twin_point_risk`（子串冒充等值）；
/// - 变量 needle ⇒ `variable_needle`（本表只分类，⛔ 不判孪生点）。
fn render_match_sites(crate_root: &Path) -> String {
    let mut rows: Vec<String> = Vec::new();
    let mut literal = 0usize;
    let mut variable = 0usize;
    let mut risk = 0usize;
    for path in source_files(&crate_root.join(SRC_ROOT)) {
        let rel = path
            .strip_prefix(crate_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path).unwrap_or_default();
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for (needle, is_literal, method) in match_sites_on_line(line) {
                let (verdict, reason): (&str, &str) = if !is_literal {
                    (
                        "variable_needle",
                        "needle 非字面量：本表只分类，⛔ 不判孪生点",
                    )
                } else if method != "contains" && method != "find" {
                    (
                        "by_design",
                        "前后缀分类器（starts_with/ends_with/strip_*）：按设计，⛔ 不冒充等值匹配",
                    )
                } else if in_test_context(&text, index) {
                    (
                        "by_design",
                        "Debug 形状断言（在 #[test] 内做子串匹配）：按设计",
                    )
                } else {
                    risk += 1;
                    (
                        "twin_point_risk",
                        "子串匹配冒充等值匹配：必须补 near-miss 对照",
                    )
                };
                if is_literal {
                    literal += 1;
                } else {
                    variable += 1;
                }
                rows.push(format!(
                    "{}|{}|{}|{}|{}|{}",
                    rel,
                    index + 1,
                    if is_literal { "literal" } else { "variable" },
                    needle,
                    verdict,
                    reason
                ));
            }
        }
    }
    rows.sort();
    let mut out = String::new();
    out.push_str("# R149／R150 匹配站点普查：`src/` 内 .contains/.find/.starts_with/.ends_with/.strip_prefix/.strip_suffix\n");
    out.push_str("# 列：file|line|kind(literal|variable)|needle|verdict(by_design|twin_point_risk|variable_needle)|reason\n");
    out.push_str("# 口径（R150，逐条与判据一致）：\n");
    out.push_str("#   ① 跨行：本表是**行式**扫描（⛔ 不跨行）。实测本 crate 折行站点 0 处\n");
    out.push_str("#      （grep -c '\\.contains($\\|\\.find($\\|\\.starts_with($\\|\\.ends_with($' = 0）。\n");
    out.push_str("#   ② 字面量定义：只认**双引号**开头；⛔ 不认原始字符串 r\"…\"／字节串 b\"…\"／字符字面量 'x'。\n");
    out.push_str(
        "#      实测本 crate 这三类 needle **各 0 处** ⇒ 当前口径**无漏判**（不是靠巧合）。\n",
    );
    out.push_str("#   ③ 注释：跳过 trim_start() 后以 // 开头的行。\n");
    out.push_str("#   ④ 行尾：text.lines() 已去 \\n／\\r；读表时走 read_evidence_text() 再归一化（R134）。\n");
    out.push_str(&format!(
        "# 合计 {} 处：literal {} ／ variable {}；其中 **twin_point_risk {} 处**\n",
        rows.len(),
        literal,
        variable,
        risk
    ));
    for row in &rows {
        out.push_str(row);
        out.push('\n');
    }
    out
}

/// **匹配站点普查表必须与当前源码逐字一致**（＋ R93 非真空地板）。
#[test]
fn the_src_match_site_census_matches_the_committed_evidence() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rendered = render_match_sites(&crate_root);
    let literal = rendered.matches("|literal|").count();
    let variable = rendered.matches("|variable|").count();
    let total = literal + variable;
    assert!(
        total >= 25,
        "匹配站点太少：{total}（地板 25）⇒ 扫描器可能退化"
    );
    assert!(
        literal >= 5,
        "字面量 needle 太少：{literal}（地板 5）⇒ 字面量判定可能失效"
    );
    assert!(
        variable >= 15,
        "变量 needle 太少：{variable}（地板 15）⇒ 分类可能退化"
    );
    let path = crate_root.join(MATCH_SITES_PATH);
    if std::env::var("R77_WRITE").is_ok() {
        fs::write(&path, &rendered).expect("写匹配站点表");
        let (back, _) = read_evidence_text(&path);
        assert_eq!(back, rendered, "匹配站点表写入后回读不一致");
        return;
    }
    let (committed, had_crlf) = read_evidence_text(&path);
    assert_eq!(
        committed, rendered,
        "匹配站点表与当前源码不一致 ⇒ 用 R77_WRITE=1 重生成（⛔ 仅在 cargo fmt 之后）"
    );
    if had_crlf {
        eprintln!("[r77] 注意：匹配站点表在盘上是 CRLF（已在比较前归一化）");
    }
    eprintln!("[r77] R149 读数：匹配站点 {total} 处（literal {literal}／variable {variable}）");
}
