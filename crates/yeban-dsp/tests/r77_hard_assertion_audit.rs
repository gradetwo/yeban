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

/// ⭐ **R164**：把源码文本归一化到**平台无关**口径。
///
/// **根因（本批 CI 实测，⛔ 不是推断）**：`windows` 腿检出的是 **CRLF** 文件；
/// `str::lines()` 只在**按行切分**时去掉行尾 `\r`，**跨行的片段**（例如断言证据串）
/// 里仍然留着 `\r` ⇒ 生成器产出的**行体**与 Linux 生成的 committed 表不同
/// （而**表头逐字相同**，正是 CI 报文里的 `left`／`right` 头部一致、行体不同的形态）。
/// ⛔ 修复**不是**在 Windows 上跳过，而是**两侧同口径归一化**。
fn normalize_source(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// ⭐ **R164**：把路径归一化到 **POSIX 分隔符**（Windows 的 `Display` 会给 `\`）。
fn normalize_path(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
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
            // ⭐ **第二个由"喂坏输入"抓到的缺陷**：`0x3f80_0000` 的**前导 `0`**（后跟 `x`）
            // 被当成整数字面量（原来的跳过逻辑只看"数字的**前一个**字符是不是 x"，管不到前导 0）。
            // 修法：数字串后面紧跟 `x`／`X` 时，**整个十六进制字面量**一起消费掉。
            if i < bytes.len() && (bytes[i] == b'x' || bytes[i] == b'X') {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                    i += 1;
                }
                continue;
            }
            let is_float = i < bytes.len() && bytes[i] == b'.';
            let type_suffix =
                i < bytes.len() && (bytes[i] == b'f' || bytes[i] == b'i' || bytes[i] == b'u');
            if !is_float && !type_suffix && i > start {
                return true;
            }
            if is_float {
                // ⭐ **R188 两臂形态当场发现的缺陷**：原先只拒了整数部分（`1` 后跟 `.`），
                // 于是**小数部分**（`5`，后跟 `)`）被当成整数字面量 ⇒ `1.5` 判成"整数计数"。
                // 修法：跳过小数部分与可选指数，⛔ 不回头重扫。
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
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
        let rel = normalize_path(&path, crate_root);
        let src = normalize_source(&fs::read_to_string(&path).unwrap_or_default());
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
    eprintln!("扫描域太小：只读到 {files} 个源文件（地板 20；实测 25）");
    eprintln!("浮点类判据太少：只扫到 {floats} 条（地板 50；实测 69）");
    eprintln!("整数计数判据太少：只扫到 {ints} 条（地板 200；实测 265）");
    eprintln!("平台感知类太少：只扫到 {platform} 条（地板 20；实测 32）—— 分类器可能退化了");
    eprintln!("可硬断言类太少：只扫到 {hard} 条（地板 20；实测 34）⇒ 分类器可能退化");

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
    eprintln!(
        "platform 类太少：{} 条（地板 20）⇒ 分类器可能退化",
        platform.len()
    );
    eprintln!("没有任何判据被识别为『已标注』⇒ 标记扫描可能失效");

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
}

#[test]
fn the_call_matching_has_near_miss_controls() {
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
        let rel = normalize_path(&path, crate_root);
        let text = normalize_source(&fs::read_to_string(&path).unwrap_or_default());
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
    eprintln!("匹配站点太少：{total}（地板 25）⇒ 扫描器可能退化");
    eprintln!("字面量 needle 太少：{literal}（地板 5）⇒ 字面量判定可能失效");
    eprintln!("变量 needle 太少：{variable}（地板 15）⇒ 分类可能退化");
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

// ---------------------------------------------------------------------------
// R118：`integer_count` 判据的界**是否界定集合大小**（四类）
// ---------------------------------------------------------------------------

/// 一条整数断言所属的类别（枚举名 ⛔ 不以变体后缀命名，避免 clippy::enum_variant_names）。
///
/// **R118 口径**：只有 [`IntegerScope::CollectionSize`] **界定集合大小** ⇒ 才算非真空下界；
/// 其余三类（值界／元素值界／条件计数器）**⛔ 不算**。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum IntegerScope {
    /// 界定**集合大小**（帧数／长度／命中总数／扫描域条目数）。
    CollectionSize,
    /// 值界：对**单个值**的上下界（如 `>= 0`、`<= 1`）。
    ValueBound,
    /// 元素值界：对**每个元素**的界（`all(|x| …)`／`any(…)`）。
    ElementValueBound,
    /// 条件计数器：数"满足条件的元素个数"，再与总数比较（比例断言）。
    ConditionCounter,
}

impl IntegerScope {
    fn as_str(self) -> &'static str {
        match self {
            IntegerScope::CollectionSize => "collection_size",
            IntegerScope::ValueBound => "value_bound",
            IntegerScope::ElementValueBound => "element_value_bound",
            IntegerScope::ConditionCounter => "condition_counter",
        }
    }
}

/// 判据体里含**整数**字面量的断言片段。
fn integer_assert_segments(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0usize;
    while let Some(p) = body[cursor..].find("assert") {
        let start = cursor + p;
        let end = body[start..]
            .find(");")
            .map_or_else(|| (start + 260).min(body.len()), |e| start + e);
        let seg = &body[start..end];
        if !seg.contains("to_bits()") && has_integer_literal(seg) {
            out.push(seg.to_string());
        }
        cursor = end + 1;
        if cursor >= body.len() {
            break;
        }
    }
    out
}

/// 给一条整数断言分类（优先级：**条件计数器 > 集合大小 > 元素值界 > 值界**）。
fn classify_integer_assert(seg: &str) -> IntegerScope {
    let has = |needle: &str| seg.contains(needle);
    let collection_markers = [
        ".len()",
        "is_empty",
        "frames",
        "count",
        "total",
        "hits",
        "seen",
        "differing",
        "n_seen",
    ];
    let counter_markers = [
        "* 10 >=", "* 100 >=", ">= total", ">= n", "ratio", "/ total",
    ];
    // ⭐ 顺序修正（第三臂抓到）：**条件计数器先判** —— 比例断言里**也会**出现 `total`／`differing`
    // 这类计数名，若先判集合大小，比例断言会被误判成"界定集合大小"。
    if counter_markers.iter().any(|m| has(m)) {
        return IntegerScope::ConditionCounter;
    }
    if collection_markers.iter().any(|m| has(m)) {
        return IntegerScope::CollectionSize;
    }
    if has(".all(") || has(".any(") || has("for ") {
        return IntegerScope::ElementValueBound;
    }
    IntegerScope::ValueBound
}

/// 审计全树的 `integer_count` 判据，返回证据表文本（含每类计数）。
fn render_integer_bound_table(crate_root: &Path) -> String {
    let mut rows: Vec<String> = Vec::new();
    let mut counts = [0usize; 4];
    for path in source_files(&crate_root.join(SRC_ROOT)) {
        let rel = normalize_path(&path, crate_root);
        let text = normalize_source(&fs::read_to_string(&path).unwrap_or_default());
        for row in classify_source(&rel, &text) {
            if row.kind != Kind::IntegerCount {
                continue;
            }
            let Some(span) = fn_spans(&text).into_iter().find(|s| s.name == row.test) else {
                continue;
            };
            let body = &text[span.start..span.end];
            let segments = integer_assert_segments(body);
            let category = segments
                .iter()
                .map(|seg| classify_integer_assert(seg))
                .min()
                .unwrap_or(IntegerScope::ValueBound);
            let evidence = segments
                .first()
                .map_or_else(String::new, |seg| seg.trim().replace('\n', " "));
            let evidence: String = evidence.chars().take(90).collect();
            counts[categorize_index(category)] += 1;
            rows.push(format!(
                "{}|{}|{}|{}",
                rel,
                row.test,
                category.as_str(),
                evidence
            ));
        }
    }
    rows.sort();
    let mut out = String::new();
    out.push_str("# R118 `integer_count` 判据的『界』分类（四类）\n");
    out.push_str("# 列：file|test|category|首条整数断言的证据（截断 90 字符）\n");
    out.push_str("# 口径（R118）：只有 collection_size **界定集合大小** ⇒ 才算非真空下界；\n");
    out.push_str("#   值界／元素值界／条件计数器 **⛔ 不算**。分类优先级：condition_counter > collection_size > element_value_bound > value_bound。\n");
    out.push_str(&format!(
        "# 合计 {} 条：collection_size {} ／ value_bound {} ／ element_value_bound {} ／ condition_counter {}\n",
        rows.len(),
        counts[0],
        counts[1],
        counts[2],
        counts[3]
    ));
    for row in &rows {
        out.push_str(row);
        out.push('\n');
    }
    out
}

fn categorize_index(b: IntegerScope) -> usize {
    match b {
        IntegerScope::CollectionSize => 0,
        IntegerScope::ValueBound => 1,
        IntegerScope::ElementValueBound => 2,
        IntegerScope::ConditionCounter => 3,
    }
}

/// 提交在仓库里的 `integer_count` 分类表。
const INTEGER_BOUND_PATH: &str = "tests/data/integer_count_categories.txt";

/// **`integer_count` 四类分类表必须与当前源码逐字一致**（＋ R93 非真空地板）。
#[test]
fn the_integer_count_categories_match_the_committed_evidence() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let rendered = render_integer_bound_table(&crate_root);
    let total = rendered.matches("|collection_size|").count()
        + rendered.matches("|value_bound|").count()
        + rendered.matches("|element_value_bound|").count()
        + rendered.matches("|condition_counter|").count();
    eprintln!("分类条目太少：{total}（地板 200）⇒ 分类器可能退化");
    let path = crate_root.join(INTEGER_BOUND_PATH);
    if std::env::var("R77_WRITE").is_ok() {
        fs::write(&path, &rendered).expect("写 integer_count 分类表");
        let (back, _) = read_evidence_text(&path);
        assert_eq!(back, rendered, "分类表写入后回读不一致");
        return;
    }
    let (committed, had_crlf) = read_evidence_text(&path);
    assert_eq!(
        committed, rendered,
        "分类表与当前源码不一致 ⇒ 用 R77_WRITE=1 重生成（⛔ 仅在 cargo fmt 之后）"
    );
    if had_crlf {
        eprintln!("[r77] 注意：分类表在盘上是 CRLF（已在比较前归一化）");
    }
    for line in rendered.lines().filter(|l| l.starts_with("# 合计")) {
        eprintln!("[r77] R118 读数：{line}");
    }
}

// ---------------------------------------------------------------------------
// ⭐ R164／R147：归一化的**配对已知红**（本机就能证明 Windows 一致性）
// ---------------------------------------------------------------------------

/// ⭐ **常驻判据（R147 配对形态）**：归一化**真的在起作用**，且生成的证据**不含平台痕迹**。
///
/// 背景（CI 实测）：`windows` 腿检出 CRLF ⇒ 生成器的**行体**里带 `\r` ⇒ 与 Linux 生成的
/// committed 表不相等（**表头相同、行体不同**）。本判据把这件事变成**本机可证**：
/// 正例（归一化后相等）＋ **配对反例**（不归一化则不等）⇒ 证明"相等"来自归一化本身，
/// ⛔ 不是因为两侧本来就一样。
#[test]
fn normalisation_has_a_paired_control() {
    // ① CRLF：归一化后必须相等。
    assert_eq!(
        normalize_source("a\r\nb\r\n"),
        normalize_source("a\nb\n"),
        "CRLF 与 LF 归一化后必须相等"
    );
    // ⭐ 配对反例：**不**归一化时必须**不**相等（否则上一条什么都没证明）。
    assert_ne!(
        "a\r\nb\r\n", "a\nb\n",
        "未归一化时 CRLF 与 LF 必须不相等 ⇒ 证明归一化是必要的"
    );

    // ② 路径分隔符：归一化后必须相等。
    // ⚠ 夹具注意（本机实测的坑）：**在 Unix 上 `\` 不是分隔符** ⇒ `strip_prefix` 会失败、
    // `unwrap_or(path)` 返回整条路径。因此这里用一个**不匹配的 root**，专门验"分隔符替换"这一步。
    let nowhere = Path::new("/nonexistent-root-xyz");
    let windows_style = Path::new("crates\\yeban-dsp\\src\\a.rs");
    assert_eq!(
        normalize_path(windows_style, nowhere),
        normalize_path(Path::new("crates/yeban-dsp/src/a.rs"), nowhere),
        "`\\` 与 `/` 归一化后必须相等"
    );
    assert_eq!(
        normalize_path(windows_style, nowhere),
        "crates/yeban-dsp/src/a.rs"
    );
    // ⭐ 配对反例：**原样**取字符串会给出 `\` ⇒ 与 POSIX 形式不等（证明替换有必要）。
    assert_ne!(
        windows_style.to_string_lossy().as_ref(),
        "crates/yeban-dsp/src/a.rs"
    );

    // ③ 端到端：三张证据表里**不得**出现 `\r`（否则 Windows 上必然不一致）。
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    for (label, rendered) in [
        (
            "hard_assertion_table",
            render_table(&audit_tree(&crate_root)),
        ),
        ("src_match_sites", render_match_sites(&crate_root)),
        (
            "integer_count_categories",
            render_integer_bound_table(&crate_root),
        ),
    ] {
        assert!(
            !rendered.contains('\r'),
            "{label} 的行体里出现了 CR ⇒ Windows 上会与 committed 表不一致"
        );
    }
}

// ---------------------------------------------------------------------------
// R146：把"结构上不可能造配对已知红"的器件类**机械登记**（⛔ 不只写在报告里）
// ---------------------------------------------------------------------------

/// ⭐ **机械登记**：`ShapingEq`／`TransientShaper` 的 `process` 把**参数按调用传入**
/// （`params: EqParams`／`params: TransientParams`）⇒ `Default` **无法携带参数差异**
/// ⇒ 对这两个类型**结构上不可能**构造"只改默认值"的配对已知红。
///
/// ⭐ **配对对照**：`Convolution::process` **不接收参数** ⇒ 它的 `Default` **可以**携带差异
/// （本会话已实测红：预配置 IR ⇒ 判据红）。两侧一起断言 ⇒ 这条分类**有判别力**，
/// ⛔ 不是"看起来像"。
#[test]
fn the_structurally_impossible_paired_reds_are_registered_mechanically() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let shaping =
        normalize_source(&fs::read_to_string(root.join("src/shaping.rs")).expect("读 shaping.rs"));
    let convolution = normalize_source(
        &fs::read_to_string(root.join("src/convolution.rs")).expect("读 convolution.rs"),
    );
    // ① 参数按调用传入：签名里出现 `params: <类型>`。
    // ⚠ 实测：`shaping.rs` 里各有 **2** 处（`process` 签名 ＋ 一个测试辅助函数），
    // 所以这里用**存在性**形态（R180：⛔ 不用"计数等于某值"的集合大小界，那会提前消解）。
    assert!(
        shaping.matches("params: EqParams").count() >= 1,
        "`ShapingEq::process` 必须把 `params: EqParams` 按调用传入（⇒ Default 无法携带参数差异）"
    );
    assert!(
        shaping.matches("params: TransientParams").count() >= 1,
        "`TransientShaper::process` 必须把 `params: TransientParams` 按调用传入"
    );
    // ② 配对对照：`Convolution::process` **不接收**参数（`&mut [f32]` 就地处理）。
    assert_eq!(
        convolution
            .matches("pub fn process(&mut self, block: &mut [f32]) -> usize")
            .count(),
        1,
        "`Convolution::process` 不该接收参数 ⇒ 它的 Default **可以**携带差异（已实测红）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ R188：扫描器的守卫**不是集合大小地板**，而是"喂坏输入必须报错、喂好输入必须通过"
// ---------------------------------------------------------------------------

/// ⭐ **R188 形态**：每个扫描器都配**两臂** ——
/// **坏输入 ⇒ 必须被拒**（⛔ 不是"数量太少"这类规模地板，那只是诊断）；
/// **好输入 ⇒ 必须被接受**。
///
/// ⚠ 规模读数（"扫到多少条"）已全部降级为 `eprintln!` 诊断（⛔ 不作失败条件）——
/// 因为"扫到很多"**不蕴涵**"扫得对"，而"喂坏输入它拒了"才蕴涵。
#[test]
fn the_scanners_reject_bad_input_and_accept_good_input() {
    // ① `classify_source`：坏输入＝没有 `#[test]` 的源码 ⇒ ⛔ 不得凭空产出条目。
    let no_tests = "fn helper() -> f32 { 1.0 }\n";
    assert!(
        classify_source("no_tests.rs", no_tests).is_empty(),
        "坏输入（无 #[test]）必须产出 0 条，⛔ 不得凭空造条目"
    );
    // 好输入 ⇒ 必须恰好扫到 1 条浮点类。
    let one_test = "#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() { assert_eq!(1.0f32.to_bits(), 0x3f80_0000); }\n}\n";
    let rows = classify_source("one_test.rs", one_test);
    assert_eq!(rows.len(), 1, "好输入必须恰好扫到 1 条");
    assert_eq!(rows[0].kind, Kind::FloatBits, "该条必须是浮点位型类");

    // ② `match_sites_on_line`：坏输入＝没有匹配的行 ⇒ ⛔ 不得产出站点。
    assert!(
        match_sites_on_line("let x = 1 + 2;").is_empty(),
        "坏输入（无匹配）必须产出 0 个站点"
    );
    // 好输入 ⇒ 字面量站点被识别为 literal，变量站点被识别为 variable（判别力）。
    let literal_sites = match_sites_on_line("if name.starts_with(\"san.\") { }");
    assert_eq!(literal_sites.len(), 1, "好输入必须恰好 1 个站点");
    assert!(literal_sites[0].1, "双引号 needle 必须判为 literal");
    let variable_sites = match_sites_on_line("if xs.contains(&needle) { }");
    assert_eq!(variable_sites.len(), 1, "变量 needle 也是 1 个站点");
    assert!(!variable_sites[0].1, "变量 needle ⛔ 不得判为 literal");
}

#[test]
fn the_scan_root_arms_reject_tests_and_see_sources() {
    // ⭐ **R223③＋R224①（构造之后的"正对照"）**：喂 token 的臂**不得抬高真扫描计数** ——
    // 本 crate 的审计只扫 `src/`，而臂与宏调用都在 `tests/` ⇒ 结构上无法污染。
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scanned = source_files(&root.join(SRC_ROOT));
    assert!(
        scanned
            .iter()
            // ⚠ **R231**：路径断言必须**先归一化** —— 否则 Windows 上是 `…\tests\…`，
            // `contains("/tests/")` **恒假** ⇒ 这条守卫会**静默失效**（不会红，但也不再防护）。
            .all(|p| !normalize_path(p, &root).contains("/tests/")),
        "扫描域里 ⛔ 不得包含 `tests/`（否则臂里的 token 会污染它自己检验的计数）"
    );
    // ⭐⭐ **R240①／R231③ 成对读数**：**同一条**路径断言在"未归一化"与"已归一化"下的结果 ——
    // 用本机合成路径证明"静默失效"确实存在，且 `normalize_path` 确实修掉它（⛔ 不是声明）。
    let windows_ish = Path::new("crates\\yeban-dsp\\tests\\x.rs");
    let raw_guard = !windows_ish.to_string_lossy().contains("/tests/");
    let norm_guard = !normalize_path(windows_ish, Path::new("crates")).contains("/tests/");
    assert!(
        raw_guard,
        "未归一化时该守卫**恒真**（⛔ 不再防护）—— 这正是 Windows 上的**静默失效**形态"
    );
    assert!(
        !norm_guard,
        "归一化后该守卫**正确为假** ⇒ 修法有效（成对读数：raw={raw_guard} / normalized={norm_guard}）"
    );

    // ⭐ **正对照（构造之后）**：扫描域必须**真的**含源文件，⛔ 否则上一条是真空的。
    // ⚠ **P0 修复**：⛔ 不能对**原始**路径用 POSIX 后缀 —— Windows 上是 `…\src\lib.rs`，
    // `ends_with("src/lib.rs")` 会**恒假** ⇒ 该正对照在 `windows` 腿上红（实测）。
    // 修法：先走 `normalize_path`（POSIX 分隔符）再判后缀。
    assert!(
        scanned
            .iter()
            .any(|p| normalize_path(p, &root) == "src/lib.rs"),
        "扫描域必须包含 `src/lib.rs` ⇒ 上一条不是真空断言（路径须先归一化）"
    );
}

#[test]
fn the_integer_scanners_reject_bad_input_and_accept_good_input() {
    // ③ `has_integer_literal`：坏输入＝**浮点**字面量 ⇒ ⛔ 不得判为整数。
    assert!(
        !has_integer_literal("assert_eq!(x, 1.5);"),
        "浮点字面量 ⛔ 不得判为整数计数"
    );
    assert!(
        !has_integer_literal("assert_eq!(x, 0x3f80_0000);"),
        "十六进制位型 ⛔ 不得判为整数计数"
    );
    // 好输入 ⇒ 纯整数必须被识别。
    assert!(
        has_integer_literal("assert_eq!(total, 96);"),
        "纯整数字面量必须被识别"
    );
}

#[test]
fn the_integer_bound_classifier_separates_the_four_classes() {
    // ④ `classify_integer_assert`：坏输入＝**值界** ⇒ ⛔ 不得判为"界定集合大小"。
    assert_eq!(
        classify_integer_assert("assert!(value >= 0);"),
        IntegerScope::ValueBound,
        "单个值的下界是 value_bound，⛔ 不是 collection_size"
    );
    // 好输入 ⇒ "计数 vs 常数"必须判为界定集合大小。
    assert_eq!(
        classify_integer_assert("assert_eq!(total, 96);"),
        IntegerScope::CollectionSize,
        "计数与常数比较必须判为 collection_size"
    );
    // 条件计数器：是"数出来的比例"，⛔ 不是集合大小。
    assert_eq!(
        classify_integer_assert("assert!(differing * 10 >= total * 9);"),
        IntegerScope::ConditionCounter,
        "比例断言必须判为 condition_counter"
    );
}

// ---------------------------------------------------------------------------
// R217(3)：器件类逐实例注入证据（**常驻** ＋ **逐条断言**）
// ---------------------------------------------------------------------------

/// 提交在仓库里的注入证据表。
const DEVICE_INJECTION_PATH: &str = "tests/data/device_default_injection_evidence.txt";

/// ⭐ **R217③ 形态**：把一次性外部注入**常驻化**，并**逐条断言**（⛔ 不是只看第一条 ——
/// libtest 在**第一条失败臂**处停止，若把 N 条臂塞进一个判据，证据链可能只有 1/N）。
///
/// ⭐ **R217②**：表头的计数必须**由同一谓词数出**（这里复用同一份 `rows`），
/// ⛔ 不重述扫描规则 —— 否则计数器会与"被计数者"用两份不同的规格（R119 的错）。
#[test]
fn the_device_injection_evidence_is_complete_and_each_row_is_asserted() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (text, _) = read_evidence_text(&root.join(DEVICE_INJECTION_PATH));
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    assert_eq!(
        rows.len(),
        15,
        "注入证据表必须恰好 15 条（13 RED ＋ 2 弱驱动），实测 {}",
        rows.len()
    );

    // ⭐ 逐条断言（每一条都独立成立，⛔ 不依赖前一条通过）。
    let mut reds = 0usize;
    for row in &rows {
        let cols: Vec<&str> = row.split('|').collect();
        assert_eq!(cols.len(), 4, "每行必须恰好 4 列：{row}");
        assert!(
            matches!(cols[2], "RED" | "GREEN_DRIVE_INSENSITIVE"),
            "读数只能是 RED 或 GREEN_DRIVE_INSENSITIVE：{row}"
        );
        assert_eq!(cols[3], "yes", "每条的还原都必须是逐字节核对通过：{row}");
        if cols[2] == "RED" {
            reds += 1;
        }
    }

    // ⭐ R217②：表头计数与数据行**由同一谓词**得出。
    let header = text
        .lines()
        .find(|l| l.starts_with("# 合计"))
        .expect("表头必须有合计行");
    assert!(
        header.contains(&format!("合计 {} 条", rows.len())),
        "表头合计必须等于数据行数（{rows:?}）：{header}"
    );
    assert!(
        header.contains(&format!("RED {reds}")),
        "表头 RED 计数必须等于按**同一谓词**数出的条数 {reds}：{header}"
    );
}

#[test]
fn the_device_injection_rows_name_real_types() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (text, _) = read_evidence_text(&root.join(DEVICE_INJECTION_PATH));
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    // ⭐ 表不能凭空写：每行的类型必须在源码里有 `impl Default for <类型>`。
    let mut src = String::new();
    for path in source_files(&root.join(SRC_ROOT)) {
        src.push_str(&normalize_source(
            &fs::read_to_string(path).unwrap_or_default(),
        ));
    }
    for row in &rows {
        let ty = row.split('|').next().unwrap_or("");
        assert!(
            src.contains(&format!("impl Default for {ty} ")),
            "源码里必须存在 `impl Default for {ty}`（证据行不得凭空写）：{row}"
        );
    }
}

// ---------------------------------------------------------------------------
// ⭐ R222①：把逐行断言**拆成独立判据**（否则 libtest 在**第一条失败臂**处停止 ⇒ 日志只覆盖 1/N）
// ---------------------------------------------------------------------------

/// 逐行断言：该类型的注入证据行必须存在、读数正确、还原为逐字节核对通过。
///
/// ⭐ **每行一个独立 `#[test]`**（由宏生成）⇒ 失败日志可覆盖 **N/N**（配合 `--no-fail-fast`）。
macro_rules! device_injection_row {
    ($name:ident, $ty:literal, $reading:literal) => {
        #[test]
        fn $name() {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let (text, _) = read_evidence_text(&root.join(DEVICE_INJECTION_PATH));
            let row = text
                .lines()
                .find(|l| l.starts_with(&format!("{}|", $ty)))
                .unwrap_or_else(|| panic!("证据表里必须有 {} 这一行", $ty));
            let cols: Vec<&str> = row.split('|').collect();
            assert_eq!(cols.len(), 4, "每行必须恰好 4 列：{row}");
            assert_eq!(cols[2], $reading, "读数不符：{row}");
            assert_eq!(cols[3], "yes", "还原必须逐字节核对通过：{row}");
        }
    };
}

device_injection_row!(noisegen_default_is_red, "NoiseGen", "RED");
device_injection_row!(adsr_default_is_red, "Adsr", "RED");
device_injection_row!(ladderfilter_default_is_red, "LadderFilter", "RED");
device_injection_row!(reverb_default_is_red, "Reverb", "RED");
device_injection_row!(convolution_default_is_red, "Convolution", "RED");
device_injection_row!(convolutionreverb_default_is_red, "ConvolutionReverb", "RED");
device_injection_row!(
    truestereoconvolution_default_is_red,
    "TrueStereoConvolution",
    "RED"
);
device_injection_row!(paramsmoother_default_is_red, "ParamSmoother", "RED");
device_injection_row!(kweighting_default_is_red, "KWeighting", "RED");
device_injection_row!(loudnessmeter_default_is_red, "LoudnessMeter", "RED");
device_injection_row!(compressor_default_is_red, "Compressor", "RED");
device_injection_row!(channelstrip_default_is_red, "ChannelStrip", "RED");
device_injection_row!(combfilter_default_is_red, "CombFilter", "RED");

// ---------------------------------------------------------------------------
// ⭐ R224③：计数口径总表（每个数字带**口径列**与**依据**）
// ---------------------------------------------------------------------------

/// 提交在仓库里的计数口径总表。
const COUNT_REGISTER_PATH: &str = "tests/data/count_register.txt";

/// ⭐ **R224③**：每个数字必须带**口径**（masked／raw／structural／log_lines／runtime_passed）
/// 与**依据**；其中**结构性**数字由**同一谓词复算**（R217②），⛔ 不许两面各写一套。
#[test]
fn the_count_register_carries_a_class_and_a_basis_for_every_number() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (text, _) = read_evidence_text(&root.join(COUNT_REGISTER_PATH));
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    let allowed = ["masked", "raw", "structural", "log_lines", "runtime_passed"];
    for row in &rows {
        let cols: Vec<&str> = row.split('|').collect();
        assert_eq!(
            cols.len(),
            4,
            "每行必须 4 列（name|value|class|basis）：{row}"
        );
        assert!(
            allowed.contains(&cols[2]),
            "口径必须是五类之一（masked/raw/structural/log_lines/runtime_passed）：{row}"
        );
        assert!(!cols[3].trim().is_empty(), "每行必须给出**依据**：{row}");
        assert!(cols[1].parse::<u64>().is_ok(), "值必须是数字：{row}");
    }
}

#[test]
fn the_count_register_numbers_are_recomputed_from_the_same_predicate() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let (text, _) = read_evidence_text(&root.join(COUNT_REGISTER_PATH));
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    // ⭐ 同一谓词复算：结构性数字必须与源码/证据表逐字对齐。
    let mut impls = 0usize;
    for path in source_files(&root.join(SRC_ROOT)) {
        let src = normalize_source(&fs::read_to_string(path).unwrap_or_default());
        impls += src.matches("impl Default for ").count();
    }
    let value = |name: &str| -> u64 {
        rows.iter()
            .find(|r| r.starts_with(name))
            .unwrap_or_else(|| panic!("总表里必须有 {name}"))
            .split('|')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap()
    };
    assert_eq!(
        value("impl_default_for"),
        impls as u64,
        "`impl Default for` 计数必须与**同一谓词**复算一致（实测 {impls}）"
    );
    for (name, path) in [
        ("rows_hard_assertion_table", TABLE_PATH),
        ("rows_integer_count_categories", INTEGER_BOUND_PATH),
        ("rows_src_match_sites", MATCH_SITES_PATH),
        ("rows_device_injection_evidence", DEVICE_INJECTION_PATH),
    ] {
        let (t, _) = read_evidence_text(&root.join(path));
        let n = t
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count() as u64;
        assert_eq!(value(name), n, "{name} 必须等于证据表的实际数据行数（{n}）");
    }
    // ⭐ 两类"外部口径"必须**点名依据**（⛔ 不是凭空写数字）。
    for name in ["raw_assert_in_src", "masked_assert_in_src"] {
        let row = rows
            .iter()
            .find(|r| r.starts_with(name))
            .unwrap_or_else(|| panic!("总表里必须有 {name}"));
        assert!(
            row.contains("grep") || row.contains("掩码"),
            "外部口径必须写明命令或掩码器：{row}"
        );
    }
}

/// 提交在仓库里的臂覆盖读数表。
const ARM_COVERAGE_PATH: &str = "tests/data/arm_coverage.txt";

/// ⭐ **E2／E3**：把"1/N"做成**可见读数**（本判据扫描**自己的源码**，按判据分区计 `assert*!`）。
///
/// ⚠ **口径与偏置（A2／A5）**：计数**当场算**（⛔ 非手数）；**按分区**报（⛔ 不跨区相加）；
/// 覆盖列必须**要么**是 `1/N`（同体内首败即停）**要么**是 `N/N`（独立判据）。
/// ⚠ **本装置的能力边界**：这是**静态**臂数，⛔ 不是运行期 `arms_ran` ——
/// "本轮实际跑到第几条"仍只能由**失败日志**给出（登记为欠项，⛔ 不冒充运行期计数器）。
#[test]
fn the_arm_coverage_register_matches_the_source() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let this_file = normalize_source(
        &fs::read_to_string(root.join("tests/r77_hard_assertion_audit.rs")).expect("读自身"),
    );
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut current = String::new();
    for line in this_file.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("fn ") {
            // ⚠ **实测抓到的缺陷**：宏体里也有 `fn $name() {`，若不过滤会把 `$name` 当成判据起点、
            // 截断其后所有判据的计数（表 10 vs 实测 7）。只接受**合法标识符**名。
            if let Some(name) = rest.split('(').next() {
                let ident = !name.is_empty()
                    && name
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
                if ident {
                    current = name.to_string();
                    counts.entry(current.clone()).or_insert(0);
                }
            }
        }
        if !current.is_empty()
            && (line.contains("assert!")
                || line.contains("assert_eq!")
                || line.contains("assert_ne!"))
        {
            *counts.entry(current.clone()).or_insert(0) += 1;
        }
    }
    let (text, _) = read_evidence_text(&root.join(ARM_COVERAGE_PATH));
    let rows: Vec<&str> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .collect();
    assert!(rows.len() >= 5, "臂覆盖表太小：{} 行（地板 5）", rows.len());
    for row in &rows {
        let cols: Vec<&str> = row.split('|').collect();
        assert_eq!(
            cols.len(),
            4,
            "每行必须 4 列（region|criterion|arms|coverage）：{row}"
        );
        assert_eq!(
            cols[0], "r77_hard_assertion_audit",
            "分区列必须点名区域：{row}"
        );
        let arms: usize = cols[2].parse().expect("arms 必须是数字");
        if cols[1].starts_with("device_injection_row") {
            assert_eq!(arms, 13, "13 条独立判据的臂数必须是 13：{row}");
            assert_eq!(
                cols[3], "13/13（每条注入 = 一条独立 #[test]）",
                "覆盖列必须是 N/N：{row}"
            );
            continue;
        }
        let fresh = *counts
            .get(cols[1])
            .unwrap_or_else(|| panic!("源码里找不到判据 {}（表不得凭空写）", cols[1]));
        assert_eq!(
            arms, fresh,
            "臂数必须与**当场扫描**一致（判据 {}：表 {arms} vs 实测 {fresh}）",
            cols[1]
        );
        assert!(
            cols[3].starts_with("1/N") || cols[3].starts_with("N/N"),
            "覆盖列必须明确 1/N 或 N/N：{row}"
        );
    }
}
