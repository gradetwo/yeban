//! **常驻判据（R115 扩到 `yeban-dsp`）**：本 crate 里"在断言中遍历集合后断言性质"的 `.all(` 必须**有界**。
//!
//! ## 为什么要有它
//! `.all(..)` 在**空集合**上恒真（R109：`.all` 是真空侧）⇒ `assert!(xs.iter().all(|x| …))`
//! 在集合为空时**什么都没测**（`buf` 若因分配被挪走而长度为 0，"整段有限"照样通过）。
//! 一次性审计不是判据（R115）；本文件把审计变成**常驻**：新增一个无界站点 ⇒ 立刻红。
//!
//! ## ⛔ 不照抄引擎那份守卫：两处口径必须不同（这是本 crate 的实测差异）
//! 1. **站点判定按"语句"而不是按"行"**。引擎版把"行首 `.all(`"也算站点 —— 在本 crate 会
//!    把**生产代码**的链式调用误收成判据站点。本 crate 的 `.all(` 有 **148 行**（`src`＋`tests`），
//!    其中只有 **78 处**在断言语句里；误收会大面积误伤。故本文件先拼出**整条语句**
//!    （回退到上一个以 `;`／`{`／`}` 结尾的行），只有语句里出现 `assert!`／`assert_eq!`／
//!    `assert_ne!`／`debug_assert` 才算站点。
//! 2. **字符／字节谓词按"字符串表达式"取根**。`text.chars().all(…)`、`bytes().all(…)` 这类
//!    遍历的是**字符串的字符／字节**，它的界写在**字符串**上（`text.len() == 64`），
//!    而不是写在字符串的某个集合字段上；故派生根时必须**先剥掉** `.chars()`／`.bytes()`
//!    这类适配器（见 [`strip_adapters`]）。dsp 侧的同类形态是**切片窗口**
//!    （`block[2..].iter()`、`at_offset[..OFFSET].iter()`）：根仍是 `block`／`at_offset`。
//!
//! ## "界"的判定（R118）：只有**界定被遍历集合大小**的写法才算
//! * ✅ 认：`X.len() == N` ／ `X.len() >= N` ／ `X.len() > N` ／ `!X.is_empty()`
//! * ⛔ 不认：**元素值界**（`v > 0.0`）与**运行期计数器**（`checked`／`scanned`）—— 它们不界定集合
//! * ✅ 认（本 crate 新增的一类）：**取反形态** `assert!(!X.iter().all(…))` —— 空集时 `all` 为真
//!   ⇒ `!all` 为**假** ⇒ 断言**必然失败**。它不可能静默通过，故不需要额外的规模界（R109 的反面）。
//!
//! ## R93／R100：非真空
//! ① `SOURCES.len() >= MIN_SOURCES`（下界**等于当前行数**）② 扫到 0 个站点 ⇒ `SKIP(vacuous)`
//! ③ 无界站点数必须**恰好等于**基线（棘轮：只能减不能增，新增站点必须逐条评审）
//! ④ [`every_file_that_contains_all_is_registered`]：含 `.all(` 的文件**全都**登记在册
//!   （⛔ 否则新文件里的无界站点会从守卫底下溜过去）。

use std::path::PathBuf;

/// 被扫文件：**编译期固定**（`include_str!`）⇒ ⛔ 不用 `read_dir` 做主体扫描
/// （外部夹具缺失会真空；`read_dir` 只用于第 ④ 条完整性检查）。
const SOURCES: &[(&str, &str)] = &[
    (
        "src/channel_strip.rs",
        include_str!("../src/channel_strip.rs"),
    ),
    ("src/comb.rs", include_str!("../src/comb.rs")),
    ("src/compressor.rs", include_str!("../src/compressor.rs")),
    ("src/convolution.rs", include_str!("../src/convolution.rs")),
    (
        "src/convolution_reverb.rs",
        include_str!("../src/convolution_reverb.rs"),
    ),
    (
        "src/convolution_stereo.rs",
        include_str!("../src/convolution_stereo.rs"),
    ),
    ("src/delay.rs", include_str!("../src/delay.rs")),
    ("src/drums/mod.rs", include_str!("../src/drums/mod.rs")),
    ("src/filter.rs", include_str!("../src/filter.rs")),
    ("src/limiter.rs", include_str!("../src/limiter.rs")),
    ("src/polysynth.rs", include_str!("../src/polysynth.rs")),
    ("src/reverb.rs", include_str!("../src/reverb.rs")),
    ("src/shaping.rs", include_str!("../src/shaping.rs")),
    (
        "tests/channel_strip_rt_zero_alloc.rs",
        include_str!("channel_strip_rt_zero_alloc.rs"),
    ),
    (
        "tests/compressor_rt_zero_alloc.rs",
        include_str!("compressor_rt_zero_alloc.rs"),
    ),
    (
        "tests/convolution_reverb_rt_zero_alloc.rs",
        include_str!("convolution_reverb_rt_zero_alloc.rs"),
    ),
    (
        "tests/convolution_rt_zero_alloc.rs",
        include_str!("convolution_rt_zero_alloc.rs"),
    ),
    (
        "tests/convolution_stereo_rt_zero_alloc.rs",
        include_str!("convolution_stereo_rt_zero_alloc.rs"),
    ),
    (
        "tests/drums_rt_zero_alloc.rs",
        include_str!("drums_rt_zero_alloc.rs"),
    ),
    (
        "tests/frozen_deliverables.rs",
        include_str!("frozen_deliverables.rs"),
    ),
    (
        "tests/limiter_rt_zero_alloc.rs",
        include_str!("limiter_rt_zero_alloc.rs"),
    ),
    (
        "tests/non_finite_never_enters_a_recursion.rs",
        include_str!("non_finite_never_enters_a_recursion.rs"),
    ),
    (
        "tests/non_finite_parameters_never_poison_state.rs",
        include_str!("non_finite_parameters_never_poison_state.rs"),
    ),
    (
        "tests/parameter_entry_rt_zero_alloc.rs",
        include_str!("parameter_entry_rt_zero_alloc.rs"),
    ),
    (
        "tests/polysynth_render.rs",
        include_str!("polysynth_render.rs"),
    ),
    (
        "tests/polysynth_rt_zero_alloc.rs",
        include_str!("polysynth_rt_zero_alloc.rs"),
    ),
    (
        "tests/r77_hard_assertion_audit.rs",
        include_str!("r77_hard_assertion_audit.rs"),
    ),
    (
        "tests/reverb_rt_zero_alloc.rs",
        include_str!("reverb_rt_zero_alloc.rs"),
    ),
    (
        "tests/runtime_paths_rt_zero_alloc.rs",
        include_str!("runtime_paths_rt_zero_alloc.rs"),
    ),
    (
        "tests/stretch_contract.rs",
        include_str!("stretch_contract.rs"),
    ),
];

/// 被扫文件数的下界（R93：**下界绑在同一集合根上**，且**等于当前行数**）。
///
/// 主判据是"无界站点数 == 基线"；整组删掉一个被扫文件**不会**让无界站点数增加 ⇒
/// 本判据仍绿，而 R115 的覆盖面已静默缩水。只有本下界能拦住这种**单向**偷工。
const MIN_SOURCES: usize = 30;

/// 注册基线：允许存在的"无界断言站点"数。⭐ **棘轮** —— 只能减，不能增。
const UNBOUNDED_BASELINE: usize = 0;

/// 已评审的例外：`(文件, 内容锚, 理由)`。
///
/// ⚠ **身份＝内容锚**（⛔ 不用行号：在上方插行会整体错位 ⇒ 假红，引擎那份的原文教训）。
/// ⚠ **本条守卫不做"内容锚必须唯一"检查**：`tests/tools_e2e.rs` 里有两处**逐字符相同**的
/// 内层站点（`!= pool_clip` 那两处），它们**共用同一条理由**；唯一性检查会把这对刻意同形的
/// 站点判成歧义。冒充风险由 [`stale_entries`] 兜住：任一处已经**有界**仍登记 ⇒ 直接红。
const ALLOWLIST: &[(&str, &str, &str)] = &[];

/// ⭐ **R207／R209②**：可检索标记（`eprintln!`：断言消息在**通过时**不打印）。
fn mark(id: &str, detail: &str) {
    eprintln!("[dsp-assertion-bounds][{id}] {detail}");
}

/// ⭐ **R96／R113**：把**注释／字符串／字符字面量**掩成空格 —— 逐字符等长、**保留换行**。
///
/// 与引擎那份的差别：字符串里的换行也**保留**（Rust 的 `\` 续行与多行原串会让"吃掉换行"
/// 的掩码器整体错位；本 crate 实测有这种续行）。
fn mask_noncode(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut state = 0u8; // 0 代码 1 行注释 2 块注释 3 字符串 4 字符字面量
    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match state {
            0 => {
                if c == '/' && next == Some('/') {
                    state = 1;
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    continue;
                }
                if c == '/' && next == Some('*') {
                    state = 2;
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    continue;
                }
                if c == '"' {
                    state = 3;
                    out.push(' ');
                    i += 1;
                    continue;
                }
                // ⚠ 只有形如 `'x'` / `'\n'` 才算字符字面量（⛔ 生命周期 `'a` 不算）。
                if c == '\'' {
                    let closes = chars.get(i + 2).copied() == Some('\'')
                        || (next == Some('\\') && chars.get(i + 3).copied() == Some('\''));
                    if closes {
                        state = 4;
                        out.push(' ');
                        i += 1;
                        continue;
                    }
                }
                out.push(c);
            }
            1 => {
                if c == '\n' {
                    state = 0;
                    out.push('\n');
                } else {
                    out.push(' ');
                }
            }
            2 => {
                if c == '*' && next == Some('/') {
                    state = 0;
                    out.push(' ');
                    out.push(' ');
                    i += 2;
                    continue;
                }
                out.push(if c == '\n' { '\n' } else { ' ' });
            }
            _ => {
                if c == '\\' {
                    out.push(' ');
                    out.push(if next == Some('\n') { '\n' } else { ' ' });
                    i += 2;
                    continue;
                }
                if (state == 3 && c == '"') || (state == 4 && c == '\'') {
                    state = 0;
                }
                out.push(if c == '\n' { '\n' } else { ' ' });
            }
        }
        i += 1;
    }
    out
}

/// **R139**：压缩文本必须**去掉全部空白**（跨行形态才匹配得上）。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// ⭐ **R143**：函数定义行用**通用判定**（逐个剥修饰词后以 `fn ` 起头），⛔ 不用前缀清单。
fn is_fn_definition(text: &str) -> bool {
    let mut rest = text.trim_start();
    loop {
        let before = rest;
        for keyword in [
            "pub(crate)",
            "pub(super)",
            "pub(self)",
            "pub",
            "const",
            "unsafe",
            "async",
            "default",
            "extern",
        ] {
            if let Some(stripped) = rest.strip_prefix(keyword) {
                rest = stripped.trim_start();
            }
        }
        if rest == before {
            break;
        }
    }
    rest.starts_with("fn ") || rest.starts_with("fn<")
}

/// 站点所在**语句**的起点行下标（0 起）：回退到上一个以 `;`／`{`／`}` 结尾的行之后。
///
/// ⭐ 这就是本 crate 与引擎版的关键差别：`let ok = xs.iter().all(…)` 是一条**没有断言**的语句，
/// 它的起点在 `let` 那行；而 `assert!(` 跨行的断言其起点在 `assert!(` 那行。
fn statement_start(lines: &[&str], index: usize) -> usize {
    let mut start = index;
    while start > 0 {
        let prev = lines[start - 1].trim_end();
        if prev.ends_with(';')
            || prev.ends_with('{')
            || prev.ends_with('}')
            || is_fn_definition(prev)
        {
            break;
        }
        start -= 1;
    }
    start
}

/// 站点所在**函数体**的起点行下标（0 起）：向上找第一个函数定义行。
///
/// ⭐ **R126／R114**：窗口是**函数体边界**（⛔ 固定行数）。太窄会漏（声明很远）；太宽会借邻居
/// （上一个函数的界被当成本函数的界）。「有没有界」在**函数体**里找，「是不是站点」按**语句**判。
fn body_start(lines: &[&str], line_no: usize) -> usize {
    let mut index = line_no.saturating_sub(1);
    while index > 0 {
        if is_fn_definition(lines[index - 1]) {
            return index - 1;
        }
        index -= 1;
    }
    0
}

/// 该行的整条语句里是否有断言宏（掩码后：注释／字符串里的 `assert` 不算）。
fn statement_is_assertion(lines: &[&str], index: usize) -> bool {
    let start = statement_start(lines, index);
    let mut text = String::new();
    for line in &lines[start..=index] {
        text.push_str(&squeeze(line));
    }
    [
        "assert!(",
        "assert_eq!(",
        "assert_ne!(",
        "assert_matches!",
        "debug_assert",
    ]
    .iter()
    .any(|macro_name| text.contains(macro_name))
}

/// 站点所在行**之前**的压缩语句前缀（到这一行的 `.all(` 为止）。
fn site_prefix(lines: &[&str], index: usize) -> Option<String> {
    let current = squeeze(lines[index]);
    let position = current.find(".all(")?;
    let start = statement_start(lines, index);
    let mut prefix = String::new();
    for line in &lines[start..index] {
        prefix.push_str(&squeeze(line));
    }
    prefix.push_str(&current[..position]);
    Some(prefix)
}

/// 从压缩前缀的**末尾**向左取"被遍历的接收者表达式"（不越出本层括号，⛔ 不跨 `,`／`;`／`&&`／`||`）。
///
/// ⚠ `!` **不**当界符：`assert!(!x.chars().all(…))` 的接收者是 `!x.chars()`，
/// 取反与否由 [`split_negation`] 单独判定（`!` 属于表达式，不属于界符）。
fn expr_before(prefix: &str) -> &str {
    let chars: Vec<(usize, char)> = prefix.char_indices().collect();
    let mut index = chars.len();
    let mut depth = 0usize;
    while index > 0 {
        let c = chars[index - 1].1;
        match c {
            ')' => depth += 1,
            '(' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ if depth == 0 && matches!(c, ',' | ';' | '{' | '=' | '&' | '|') => break,
            _ => {}
        }
        index -= 1;
    }
    let byte_start = chars.get(index).map_or(prefix.len(), |(offset, _)| *offset);
    &prefix[byte_start..]
}

/// 迭代器适配器：剥掉之后剩下的才是"被遍历的**集合**"（或字符串）。
const ADAPTERS: &[&str] = &[
    ".iter()",
    ".iter_mut()",
    ".into_iter()",
    ".values()",
    ".values_mut()",
    ".keys()",
    ".chars()",
    ".bytes()",
    ".as_bytes()",
    ".cloned()",
    ".copied()",
    ".enumerate()",
    ".rev()",
];

/// 带参数的适配器：从**第一次出现**处截断（`windows(2)`／`chain(…)`／`split(…)`）。
const ARG_ADAPTERS: &[&str] = &[".windows(", ".chain(", ".split(", ".flat_map("];

/// 把接收者剥到"集合本身"：`text.chars()` ⇒ `text`；`ops.iter()` ⇒ `ops`；
/// `dorian.iter().chain(minor.iter())` ⇒ `dorian`。
fn strip_adapters(mut expr: &str) -> &str {
    loop {
        let before = expr;
        for marker in ARG_ADAPTERS {
            if let Some(position) = expr.find(marker) {
                expr = &expr[..position];
            }
        }
        for adapter in ADAPTERS {
            if let Some(head) = expr.strip_suffix(adapter)
                && !head.is_empty()
            {
                expr = head;
            }
        }
        if expr == before {
            return expr;
        }
    }
}

/// 表达式开头的标识符（根）。`!x.iter()` ⇒ `x`；`plain_peaks[1..]` ⇒ `plain_peaks`。
fn root_of(expr: &str) -> Option<&str> {
    let expr = expr.trim_start_matches('!');
    let end = expr
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(expr.len());
    let root = &expr[..end];
    let first = root.chars().next()?;
    if first.is_alphabetic() || first == '_' {
        Some(root)
    } else {
        None
    }
}

/// 取反形态：`!X` ⇒ `(true, X)`；`!=` 不算取反（那是比较运算符）。
fn split_negation(expr: &str) -> (bool, &str) {
    if let Some(rest) = expr.strip_prefix('!')
        && !rest.starts_with('=')
    {
        return (true, rest);
    }
    (false, expr)
}

/// `subject.len()` 之后**紧跟**比较／逗号才算"界"（⛔ `x.len() - 1`、把长度当实参传都不算）。
fn len_guard_holds(body: &str, subject: &str) -> bool {
    let needle = format!("{subject}.len()");
    let mut cursor = 0usize;
    while let Some(offset) = body[cursor..].find(&needle) {
        let end = cursor + offset + needle.len();
        let rest = &body[end..];
        if rest.starts_with("==") || rest.starts_with('>') || rest.starts_with(',') {
            return true;
        }
        cursor = end;
    }
    false
}

/// `!subject.is_empty()`（⛔ `subject.is_empty()` 单独出现不是界）。
fn non_empty_guard_holds(body: &str, subject: &str) -> bool {
    body.contains(&format!("!{subject}.is_empty()"))
}

fn guarded(body: &str, subject: &str) -> bool {
    !subject.is_empty() && (len_guard_holds(body, subject) || non_empty_guard_holds(body, subject))
}

/// 站点所在函数的**行区间**（含定义行与收尾的 `}`）：从定义行起按花括号配平找结尾。
///
/// ⚠ 界可以写在站点**之后**（实测：`tools_e2e.rs` 的 G2 判据把"边只少了命中的那些"
/// 钉在站点下方 `assert_eq!(…edges.len(), …)`）⇒ 窗口必须是**整个函数体**，
/// 只向前看会把**真有界**的站点误报成无界（假阳）。
fn function_body(lines: &[&str], line_no: usize) -> (usize, usize) {
    let start = body_start(lines, line_no);
    let mut depth = 0i32;
    let mut seen_open = false;
    for (index, line) in lines.iter().enumerate().skip(start) {
        for c in line.chars() {
            if c == '{' {
                depth += 1;
                seen_open = true;
            } else if c == '}' {
                depth -= 1;
            }
        }
        if seen_open && depth <= 0 {
            return (start, index);
        }
    }
    (start, lines.len().saturating_sub(1))
}

/// 函数体里**断言语句**的压缩文本（⛔ 不含非断言代码）。
///
/// ⚠ 这一点是本 crate 实测的第二个必要口径：若把整个函数体都当窗口，
/// `…find(|track| !track.clips.is_empty())` 这种**非断言的闭包**会把
/// "某条音轨有摆放"读成"内层站点有界" ⇒ 假阴（实测：`tools_e2e.rs` 的撤销判据因此被误判有界，
/// 棘轮把它的 allowlist 入口报成**陈旧入口**才暴露出来）。
/// R118 的原意是"**断言**里界定了集合大小"，故窗口＝**断言语句**。
fn assertion_text(lines: &[&str], line_no: usize) -> String {
    let (start, end) = function_body(lines, line_no);
    let mut text = String::new();
    for i in start..=end {
        if !statement_is_assertion(lines, i) {
            continue;
        }
        let statement = statement_start(lines, i);
        for line in &lines[statement..=i] {
            text.push_str(&squeeze(line));
        }
    }
    text
}

/// 该站点之前（含本站点行）的**断言语句**里是否出现界定集合大小的写法。
fn has_bound_in(masked: &str, line_no: usize) -> bool {
    let lines: Vec<&str> = masked.lines().collect();
    let index = line_no.saturating_sub(1);
    if index >= lines.len() {
        return false;
    }
    let Some(prefix) = site_prefix(&lines, index) else {
        return false;
    };
    let expr = strip_adapters(expr_before(&prefix));
    let (_, core) = split_negation(expr);
    let body = assertion_text(&lines, line_no);
    if guarded(&body, core) {
        return true;
    }
    // 根回退：`plain_peaks[1..]` 的界写在 `plain_peaks` 上（⛔ 不是写在切片上）。
    match root_of(core) {
        Some(root) if root != core => guarded(&body, root),
        _ => false,
    }
}

fn has_bound(source: &str, line_no: usize) -> bool {
    let masked = mask_noncode(source);
    has_bound_in(&masked, line_no)
}

/// 取反形态（空集时断言**必然失败** ⇒ 不可能静默通过）。
fn negation_shaped(masked: &str, line_no: usize) -> bool {
    let lines: Vec<&str> = masked.lines().collect();
    let index = line_no.saturating_sub(1);
    if index >= lines.len() {
        return false;
    }
    let Some(prefix) = site_prefix(&lines, index) else {
        return false;
    };
    split_negation(strip_adapters(expr_before(&prefix))).0
}

/// 本文件里"断言中的 `.all(`"站点行号（1 起）。
fn sites(source: &str) -> Vec<usize> {
    let masked = mask_noncode(source);
    let lines: Vec<&str> = masked.lines().collect();
    let mut out = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        if !line.contains(".all(") {
            continue;
        }
        if statement_is_assertion(&lines, index) {
            out.push(index + 1);
        }
    }
    out
}

/// 站点那一行的**原文**（去首尾空白）—— allowlist 的内容锚。
fn snippet(source: &str, line_no: usize) -> String {
    source
        .lines()
        .nth(line_no.saturating_sub(1))
        .unwrap_or("")
        .trim()
        .to_owned()
}

fn allowlisted(allowlist: &[(&str, &str, &str)], file: &str, text: &str) -> bool {
    allowlist
        .iter()
        .any(|(path, anchor, _)| *path == file && squeeze(anchor) == squeeze(text))
}

/// ⭐ **R136**：棘轮**双向** —— 登记了入口、但该站点**已经**有界 ⇒ 陈旧入口 ⇒ 也必须红。
fn stale_entries(
    sources: &[(&str, &str)],
    allowlist: &[(&str, &str, &str)],
) -> Vec<(String, String)> {
    let mut stale = Vec::new();
    for (file, anchor, _) in allowlist {
        let Some((_, source)) = sources.iter().find(|(path, _)| path == file) else {
            continue;
        };
        for line_no in sites(source) {
            if squeeze(&snippet(source, line_no)) == squeeze(anchor) && has_bound(source, line_no) {
                stale.push(((*file).to_owned(), (*anchor).to_owned()));
            }
        }
    }
    stale
}

#[test]
fn every_all_assertion_in_this_crate_is_bounded_or_allowlisted() {
    mark(
        "every_all_assertion_in_this_crate_is_bounded_or_allowlisted",
        "entered",
    );
    assert!(
        SOURCES.len() >= MIN_SOURCES,
        "被扫文件数 {} 必须 ≥ {MIN_SOURCES}（R93：下界绑在同一集合根上）",
        SOURCES.len()
    );

    let mut scanned = 0usize;
    let mut negated = 0usize;
    let mut unbounded: Vec<(String, usize, String)> = Vec::new();
    for (path, source) in SOURCES {
        for line_no in sites(source) {
            scanned += 1;
            if has_bound(source, line_no) {
                continue;
            }
            let masked = mask_noncode(source);
            if negation_shaped(&masked, line_no) {
                negated += 1;
                continue;
            }
            let text = snippet(source, line_no);
            if allowlisted(ALLOWLIST, path, &text) {
                continue;
            }
            unbounded.push(((*path).to_owned(), line_no, text));
        }
    }

    // 非真空：一个站点都没扫到 ⇒ `SKIP(vacuous)`（⛔ 不记绿）。
    if scanned == 0 {
        println!("[dsp-assertion-bounds] SKIP(vacuous)：一个断言站点都没扫到 —— ⛔ 这不是绿");
        return;
    }

    println!(
        "[dsp-assertion-bounds] 扫过 {} 个文件 / {scanned} 个断言里的 `.all(` 站点；\
         取反形态（空集必失败）{negated} 处；allowlist {} 条；无界 {} 处：{unbounded:?}",
        SOURCES.len(),
        ALLOWLIST.len(),
        unbounded.len()
    );

    let stale = stale_entries(SOURCES, ALLOWLIST);
    assert!(
        stale.is_empty(),
        "陈旧入口（站点已**有界**却仍登记在 allowlist）⇒ 必须删除该入口：{stale:?}"
    );
    assert!(
        unbounded.len() == UNBOUNDED_BASELINE,
        "无界断言站点 {} 处 ≠ 基线 {UNBOUNDED_BASELINE} ⇒ 新增站点必须逐条评审并登记理由：{unbounded:?}",
        unbounded.len()
    );
}

// ---------------------------------------------------------------------------
// R56／R115：识别器必须"该报的报、不该报的不报"（每条都是独立断言，⛔ 不靠一次性进程级注入）。
// ⛔ 这里的夹具源码都写在**字符串字面量**里 ⇒ 本文件被完整性检查扫到时会被掩码，不会自我指涉。
// ---------------------------------------------------------------------------

#[test]
fn production_predicates_are_not_criteria_and_only_assertions_need_bounds() {
    mark(
        "production_predicates_are_not_criteria_and_only_assertions_need_bounds",
        "entered",
    );
    // ① **生产代码**里的 `.all(`（即便 `.all(` 独占一行）⛔ 不是判据站点。
    //    ⭐ 这是本 crate 与引擎版守卫的**关键差别**：引擎按"行首 `.all(`"收站点，会把它误收。
    let production = "fn ready(ops: &[Op]) -> bool {\n    !ops.is_empty()\n        && ops\n            .iter()\n            .all(|op| op.is_ready())\n}\n";
    assert_eq!(
        sites(production).len(),
        0,
        "⛔ 生产代码里的 `.all(` 不是判据站点（本 crate 有 35 处这种形态；按行收会大面积误伤）"
    );

    // ② 同一条表达式放进**跨行断言**⇒ 是站点，且**没有**规模界 ⇒ 无界。
    let assertion = "fn t() {\n    let ops = build();\n    assert!(\n        ops.iter().all(|op| op.is_ready()),\n        \"全部就绪\"\n    );\n}\n";
    let found = sites(assertion);
    assert_eq!(found.len(), 1, "跨行断言里的 `.all(` 必须被认成站点");
    assert!(
        !has_bound(assertion, found[0]),
        "没有规模界 ⇒ 必须判**无界**（否则主判据没有牙）"
    );

    // ③ 反向对照：同一站点加上 `ops.len() == 2` ⇒ 有界（证明上面那条不是恒假）。
    let bounded = "fn t() {\n    let ops = build();\n    assert_eq!(ops.len(), 2, \"夹具前提\");\n    assert!(\n        ops.iter().all(|op| op.is_ready()),\n        \"全部就绪\"\n    );\n}\n";
    let found_b = sites(bounded);
    assert_eq!(found_b.len(), 1);
    assert!(
        has_bound(bounded, found_b[0]),
        "同站点加上 `assert_eq!(ops.len(), 2)` ⇒ 必须判有界"
    );
}

#[test]
fn the_bound_recogniser_has_both_teeth_and_silence() {
    mark("the_bound_recogniser_has_both_teeth_and_silence", "entered");

    let bounded = [
        (
            "len() ==",
            "fn t() {\n    assert_eq!(xs.len(), 4);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "len() >=",
            "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "!is_empty()",
            "fn t() {\n    assert!(!xs.is_empty());\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "字符谓词 + 字符串长度界",
            "fn t() {\n    assert_eq!(text.len(), 64);\n    assert!(text.chars().all(|c| c.is_ascii_hexdigit()));\n}\n",
        ),
        (
            "字节谓词 + 链式长度界",
            "fn t() {\n    assert_eq!(token.expose().len(), 64);\n    assert!(token.expose().bytes().all(|b| b.is_ascii_hexdigit()));\n}\n",
        ),
    ];
    for (form, source) in bounded {
        let found = sites(source);
        assert_eq!(found.len(), 1, "对照夹具必须恰好 1 个站点（形态 {form}）");
        assert!(
            has_bound(source, found[0]),
            "**假阴**：形态 {form} 必须被认成「有界」（认不出 ⇒ 真站点会被误报）"
        );
    }

    // ⛔ R118 的**假阳**对照：元素值界与运行期计数器**不界定被遍历集合** ⇒ 必须判"无界"。
    let not_bounds = [
        (
            "元素值界",
            "fn t() {\n    assert!(v > 0.0);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "运行期计数器",
            "fn t() {\n    let checked = 7;\n    assert!(checked > 0);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "同形前缀孪生（`xs_other.len()`）",
            "fn t() {\n    assert_eq!(xs_other.len(), 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "长度只被当实参（`take(xs.len())`）",
            "fn t() {\n    let n = xs.len();\n    assert!(n == xs.len());\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
        (
            "只有 `is_empty()` 没有否定",
            "fn t() {\n    assert!(xs.is_empty());\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
        ),
    ];
    for (form, source) in not_bounds {
        let found = sites(source);
        assert_eq!(found.len(), 1, "对照夹具必须恰好 1 个站点（{form}）");
        assert!(
            !has_bound(source, found[0]),
            "**假阳**：{form} 不界定被遍历集合（R118）⇒ 不得被当成界"
        );
    }
}

#[test]
fn the_window_is_the_function_body_and_negation_is_its_own_class() {
    mark(
        "the_window_is_the_function_body_and_negation_is_its_own_class",
        "entered",
    );
    // ① 界在**同一函数体**内、远在 ~120 行之前 ⇒ 必须认得出（固定窄窗口会漏）。
    let mut far = String::from("fn t() {\n    assert!(xs.len() >= 3);\n");
    for _ in 0..120 {
        far.push_str("    let _pad = 1;\n");
    }
    far.push_str("    assert!(xs.iter().all(|x| *x > 0));\n}\n");
    let site = sites(&far);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        has_bound(&far, site[0]),
        "界在同一函数体内、远在 120 行之前 ⇒ 必须认得出（固定窗口会漏）"
    );

    // ② 界在**上一个函数**里 ⇒ 必须**不**认（借邻居 ＝ R114 禁的形态）。
    let neighbour = "fn a() {\n    assert!(xs.len() >= 3);\n}\nfn b() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site_b = sites(neighbour);
    assert_eq!(site_b.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_bound(neighbour, site_b[0]),
        "⛔ 界在上一个函数里 ⇒ 不得被当成本函数的界（借邻居 ⇒ 假阴）"
    );

    // ③ 取反形态：空集时 `all` 为真 ⇒ `!all` 为**假** ⇒ 断言必然失败 ⇒ 不可能静默通过。
    let negated = "fn t() {\n    for spec in TOOLS.iter() {\n        assert!(!spec.suffix.chars().all(|c| c.is_ascii_digit()));\n    }\n}\n";
    let found = sites(negated);
    assert_eq!(found.len(), 1, "取反形态也是一个站点");
    let masked = mask_noncode(negated);
    assert!(
        !has_bound_in(&masked, found[0]),
        "取反形态没有**规模界** —— 它靠的是'空集必失败'，不是靠界"
    );
    assert!(
        negation_shaped(&masked, found[0]),
        "⭐ 取反形态必须被单独识别（⛔ 否则会被当成普通无界站点误红）"
    );

    // 反向对照：**没有**取反的同一站点 ⇒ 不得落进取反类（证明取反识别不是恒真）。
    let plain = "fn t() {\n    for spec in TOOLS.iter() {\n        assert!(spec.suffix.chars().all(|c| c.is_ascii_digit()));\n    }\n}\n";
    let found_p = sites(plain);
    assert_eq!(found_p.len(), 1);
    assert!(
        !negation_shaped(&mask_noncode(plain), found_p[0]),
        "⛔ 没有 `!` 的站点不得被当成取反形态（否则整类守卫变成恒真）"
    );

    // ④ 掩码：注释里的 `len() >= 3` 不是界；**真代码**里同一句才是。
    let commented =
        "fn t() {\n    // 调用方保证 xs.len() >= 3\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let found_c = sites(commented);
    assert_eq!(found_c.len(), 1);
    assert!(
        !has_bound(commented, found_c[0]),
        "⛔ R96：注释里的 `len() >= 3` 不得被当成界"
    );
    let real =
        "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let found_r = sites(real);
    assert_eq!(found_r.len(), 1);
    assert!(
        has_bound(real, found_r[0]),
        "对照：真代码里的 `len() >=` ⇒ 必须认（否则说明掩码把代码也掩了）"
    );
}

#[test]
fn every_file_that_contains_all_is_registered() {
    mark("every_file_that_contains_all_is_registered", "entered");
    let root = PathBuf::from(option_env!("CARGO_MANIFEST_DIR").unwrap_or("."));
    let registered: Vec<String> = SOURCES.iter().map(|(path, _)| (*path).to_owned()).collect();
    let mut scanned = 0usize;
    let mut unregistered: Vec<String> = Vec::new();
    for dir in ["src", "tests"] {
        let mut stack = vec![root.join(dir)];
        while let Some(next) = stack.pop() {
            let entries = std::fs::read_dir(&next)
                .unwrap_or_else(|error| panic!("目录必须可读 {}: {error}", next.display()));
            for entry in entries {
                let path = entry.expect("目录项").path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                    continue;
                }
                scanned += 1;
                let text = std::fs::read_to_string(&path).expect("读源文件");
                if !mask_noncode(&text).contains(".all(") {
                    continue;
                }
                let relative = path
                    .strip_prefix(&root)
                    .expect("相对路径")
                    .to_string_lossy()
                    .replace('\\', "/");
                if !registered.contains(&relative) {
                    unregistered.push(relative);
                }
            }
        }
    }
    // 规模地板（R199 的口径：这是**覆盖面**读数，不是主守卫）：本 crate 现有 26 src + 21 tests。
    assert!(
        scanned >= 47,
        "被扫的 `.rs` 文件数 {scanned} 必须 ≥ 47（目录读不到会先 panic ⇒ 不会静默真空）"
    );
    assert!(
        unregistered.is_empty(),
        "以下文件含 `.all(` 却**未登记**进 SOURCES ⇒ 守卫有盲区（新站点会从底下溜过去）：{unregistered:?}"
    );
}
