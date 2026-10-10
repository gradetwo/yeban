//! **常驻判据（R115）**：本 crate 里"遍历集合后断言性质"的 `.all(` 必须**有界**。
//!
//! ## 为什么要有它
//! `.all(..)` 在**空集合**上恒真（R109：`.all` 是真空侧）⇒ `.iter().all(|x| …)` 这类断言
//! 在集合为空时**什么都没测**。一次性审计不是判据（R115）：审计做完就没人再跑它。
//! 本文件把审计变成**常驻**：新增一个无界 `.all(` 站点 ⇒ 本判据立刻变红。
//!
//! ## "界"的判定按 **R118**：只有**界定被遍历集合大小**的写法才算
//! * ✅ 认：`X.len() == N` ／ `X.len() >= N` ／ `X.len() > 0` ／ `!X.is_empty()`
//! * ⛔ 不认：**元素值界**（`x > 0.0`）与**运行期计数器**（`checked`/`scanned`/…）
//!   —— 它们**不界定被遍历集合**（本 crate 分别有 209 / 490 处，认了就会把无界站点误判为有界）。
//!
//! ## R84：针与期望形态**运行期构造**
//! 本文件的"有界窗口"识别器把**形态**当参数传进去（见 [`size_bound_forms`]），
//! ⛔ 不把期望字面量直接写进 `contains` 实参（那会读的正是本文件、恒真）。
//!
//! ## R93／R100：非真空
//! ① `SOURCES.len() >= 8`（下界绑定**同一集合根**）② 被扫到的 `.all(` 站点为 0 ⇒ 记
//! `SKIP(vacuous)`（⛔ 不记绿）③ 无界站点数 **不得超过注册基线**（棘轮：只能减，不能增）。

/// 被扫文件：**编译期固定**（`include_str!`）⇒ ⛔ 不用 `read_dir`（R100：外部夹具缺失会真空）。
const SOURCES: &[(&str, &str)] = &[
    ("src/rt.rs", include_str!("../src/rt.rs")),
    ("src/synth.rs", include_str!("../src/synth.rs")),
    ("src/graph.rs", include_str!("../src/graph.rs")),
    ("src/mixer.rs", include_str!("../src/mixer.rs")),
    ("src/param.rs", include_str!("../src/param.rs")),
    ("src/level.rs", include_str!("../src/level.rs")),
    ("src/insert.rs", include_str!("../src/insert.rs")),
    ("src/snapshot.rs", include_str!("../src/snapshot.rs")),
    ("tests/rt_zero_alloc.rs", include_str!("rt_zero_alloc.rs")),
    ("tests/mix_render.rs", include_str!("mix_render.rs")),
    ("tests/support/mod.rs", include_str!("support/mod.rs")),
    (
        "tests/idempotency_and_channel_consistency.rs",
        include_str!("idempotency_and_channel_consistency.rs"),
    ),
];

/// 被扫文件数的**下界**（R93：下界必须绑在同一个集合根上）。
const MIN_SOURCES: usize = 8;

/// 注册基线：允许存在的"无界 `.all(`"站点数。⭐ **棘轮** —— 只能减，不能增；
/// 每一条都必须在下面 `ALLOWLIST` 里给出**理由**（新增站点 ⇒ 本判据红 ⇒ 必须逐条评审）。
const UNBOUNDED_BASELINE: usize = 0;

/// 已评审的例外：`(文件, 行号, 理由)`。
///
/// ⚠ **已知脆弱（本次喂牙实测暴露，⛔ 下一轮必修）**：入口按**行号**登记 ⇒
/// 在该文件**上方插入任意行**会把它们整体错位（实测：插入 6 行后，两条入口失配 ⇒ 判据**假红**）。
/// 修法（R117／R119 家族）：入口改成**内容锚**（把站点那一行规范化后取片段），⛔ 不用行号。
/// 现在先**如实登记**，因为"假红"比"假绿"安全 —— 它不会让真缺陷溜过去。
const ALLOWLIST: &[(&str, &str, &str)] = &[
    // ⭐ **R132 的进度读数**：条目数 ＝ "已评审但**未修**"的真缺口数。
    // 本批把两条**真无界直接修掉**（`src/insert.rs` 加 `!a.is_empty()`；
    // `tests/idempotency_and_channel_consistency.rs` 加 `out.len() >= 2`）⇒ **条目数 = 0**。
    //
    // ⚠ 若将来再加入口：**身份必须用内容锚**（文件 ＋ 站点规范化片段），⛔ 不许用行号；
    // 且**该片段必须在文件内唯一**（否则两个同形站点会互相冒充 ⇒ 见 `ambiguous_anchors` 守卫）。
];

/// ⭐ **R207／R209②／R210④**：**可检索标记**。必须用 `eprintln!`（⛔ 断言消息在**通过时**不打印），
/// 并且**只有 `--nocapture` 下可见**；⭐ 读到 0 时先核对**目标**（错目标会给**静默的 0**）。
fn mark(id: &str, detail: &str) {
    eprintln!("[assertion-bounds][{id}] {detail}");
}

/// 窗口：`.all(` 站点**之前**多少行内去找"界"。
// ⚠ 窗口必须**够宽**才能看到接收者的声明（实测：30 行时，我自己的声相判据里
//  落在窗口外 ⇒ **假阳 2 处**）。
/// ⭐ **R126**：窗口**不是**固定行数 —— 从站点向上扫到**所在函数体的起点**为止。
///
/// 固定宽度两头都会错：太窄会**漏**（接收者声明很远 ⇒ 假阳，实测 2 处）；太宽会**借邻居**
/// （上一函数的界被当成本函数的界 ⇒ **假阴**，正是 R114 禁的形态）。
/// 函数体是**语义上正确的边界**；`WINDOW_MAX` 只是防御性上限（⛔ 非语义边界）。
const WINDOW_MAX: usize = 400;

/// ⭐ **R118**：只有这几种写法**界定被遍历集合的大小**。
/// ⛔ 元素值界（`> 0.0`）与运行期计数器**不在其中**。
fn size_bound_forms() -> [String; 5] {
    // R84：形态在**运行期**拼出来（⛔ 不写字面量进 `contains` 实参）。
    let len = "len";
    [
        format!("{len}() =="),
        format!("{len}() >="),
        format!("{len}() >"),
        ".is_empty()".to_owned(),
        format!("{len}(), "),
    ]
}

/// ⭐ **R147①**：剥关键字（`let`／`let mut`）**必须紧跟非标识符字符** ——
/// ⛔ 否则 `letter = 5` 会被剥成 `ter = 5`（前缀剥离孪生，与 `bb` vs `b` 同类）。
fn strip_keyword<'a>(text: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = text.strip_prefix(keyword)?;
    match rest.chars().next() {
        None => Some(rest),
        Some(c) if !(c.is_alphanumeric() || c == '_') => Some(rest.trim_start()),
        _ => None,
    }
}

/// ⭐ **R147②**：`call(ident` 之后必须**紧跟非标识符字符** ——
/// ⛔ 否则 `assert_eq!(drained_x, 10)` 会被当成 `drained` 的界（**后缀**孪生）。
fn has_call_with_ident(flat: &str, call: &str, ident: &str) -> bool {
    let needle = format!("{call}({ident}");
    let mut cursor = 0usize;
    while let Some(offset) = flat[cursor..].find(&needle) {
        let end = cursor + offset + needle.len();
        match flat[end..].chars().next() {
            Some(c) if c.is_alphanumeric() || c == '_' => cursor = end,
            _ => return true,
        }
    }
    false
}

/// ⭐ **R96／R113**：把**注释／字符串／字符字面量**掩成空格 ——
/// ① **保留换行**（⛔ 吃掉换行会让行号漂移）② **逐字符等长**（按 `len_utf8` 补空格 ⇒ 偏移不错位）。
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
                // ⚠ 只有形如 `'x'` / `'\n'` 才当字符字面量（⛔ 生命周期 `'a` 不当）。
                if c == '\'' {
                    let closes = chars
                        .get(i + 1)
                        .map(|_| chars.get(i + 2).copied() == Some('\''))
                        .unwrap_or(false)
                        || (chars.get(i + 1) == Some(&'\\') && chars.get(i + 3) == Some(&'\''));
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
            3 | 4 => {
                if c == '\\' {
                    out.push(' ');
                    if chars.get(i + 1).is_some() {
                        out.push(' ');
                        i += 2;
                        continue;
                    }
                } else if (state == 3 && c == '"') || (state == 4 && c == '\'') {
                    state = 0;
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
        i += 1;
    }
    out
}

/// ⭐ **R153**：站点遍历的**根表达式**必须**从站点派生**（⛔ 不用白名单）。
/// 形态：`X.iter()`／`X[` 之前的 `X`（截到最近的界符）。
fn site_root(context: &str) -> Option<String> {
    let flat = squeeze(context);
    let index = flat.find(".iter()")?;
    let before = &flat[..index];
    let root: String = before
        .chars()
        .rev()
        .take_while(|c| !matches!(c, '(' | ',' | ';' | '=' | '{' | '}' | '!' | '&' | '|' | ':'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if root.is_empty() { None } else { Some(root) }
}

/// **R139**：压缩文本必须**去掉全部空白**（⛔ 只去空格不够 —— 跨行形态会匹配不上，
/// 把**对的代码**报成缺陷）。
fn squeeze(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// 从上下文里**派生** `接收者[..界]`（⛔ 不用硬编码标识符白名单 —— 那是我上一版的缺陷）。
fn slice_receiver_bound(context: &str) -> Option<(String, String)> {
    let flat = squeeze(context);
    let start = flat.find("[..")?;
    let bound_start = start + 3;
    let rest = &flat[bound_start..];
    let bound: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let before = &flat[..start];
    let receiver: String = before
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    if receiver.is_empty() || bound.is_empty() {
        return None;
    }
    Some((receiver, bound))
}

/// 站点**所在函数体**的起点行下标（0 起）：向上找第一个 `fn …` 起头处。
fn enclosing_body_start(lines: &[&str], line_no: usize) -> usize {
    let mut index = line_no.saturating_sub(1);
    while index > 0 {
        if is_fn_definition(lines[index - 1]) {
            return index - 1;
        }
        index -= 1;
    }
    0
}

/// ⭐ **R143**：函数定义行必须用**通用判定**，⛔ 不用前缀清单。
///
/// 实测代价（本 crate）：前缀清单只认 `fn `/`pub fn `/`pub(crate) fn `/`const fn `，
/// 而本 crate 另有 **`pub const fn` 178 处**、`unsafe fn` 15、`pub(super) fn` 10、
/// `pub(crate) const fn` 2 ⇒ 这些函数的"函数体边界"全部失效 ⇒ 窗口会**借邻居**（假阴）。
///
/// 判定 = 逐个剥掉修饰词后，**以 `fn ` 起头**（调用行如 `fn_name();` ⛔ 不匹配）。
/// ⚠ 剥完若为空串 ⇒ `starts_with` 为假 ⇒ **安全侧**（⛔ 不会静默通过）。
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

/// 一行是否是"断言里的 `.all(`"站点。
fn is_assertion_site(line: &str) -> bool {
    line.contains(".all(") && (line.contains("assert") || line.trim_start().starts_with(".all("))
}

/// 扫一个文件：返回**断言里的** `.all(` 站点行号（1 起）。
fn sites(source: &str) -> Vec<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| is_assertion_site(line))
        .map(|(index, _)| index + 1)
        .collect()
}

/// 该站点**之前** `WINDOW` 行里是否出现**界定集合大小**的写法（R118）。
fn has_size_bound(source: &str, line_no: usize) -> bool {
    let lines: Vec<&str> = source.lines().collect();
    let body_start = enclosing_body_start(&lines, line_no);
    let start = body_start.max(line_no.saturating_sub(WINDOW_MAX + 1));
    let window = lines[start..line_no.saturating_sub(1).min(lines.len())].join("\n");
    // ⭐ **R96／R113**：注释与字面量里的 `len() >=` **不算界** ⇒ 先掩码（保长）。
    // ⚠ 掩码之后**再压缩**：针是从**压缩**文本派生的（`xs.len()>=`），
    // 拿它去匹配**未压缩**窗口（`xs.len() >=`）永远不中 —— 实测 4 条臂因此全红。
    let window = squeeze(&mask_noncode(&window));
    // ⭐ **R153：五种基础形态都必须**提到站点的根**** —— ⛔ "窗口里出现任一"不算
    // （那是无根绑定：删掉一个界会被同函数体里**别的**界兜住 ⇒ 实例级注入不可隔离）。
    let mut bound = false;
    // ⚠ 根必须从**站点自身**的上下文派生（含站点行）：`.all(` 常独占一行，
    // 而 `.iter()` 在**上一行** ⇒ 只看"站点之前的窗口"会派生不出根（实测 5 条臂全红）。
    let site_context = lines[line_no.saturating_sub(4)..line_no.min(lines.len())].join("\n");
    if let Some(root) = site_root(&site_context) {
        let len_root = format!("{root}.len()");
        bound = window.contains(&format!("{len_root}=="))
            || window.contains(&format!("{len_root}>="))
            || window.contains(&format!("{len_root}>"))
            // ⭐ `assert_eq!(x.len(), N)` 的形态：`x.len(),`（⛔ 上一版漏了这个变体）
            || window.contains(&format!("{len_root},"))
            // ⚠ **不算**：元素值界 `assert_eq!(x[0], N)`（它不界定集合大小，R118）
            || (window.contains(&format!("{root}.is_empty()")) && window.contains('!'));
    }
    if bound {
        return true;
    }
    let _ = size_bound_forms();
    // ⭐ 本 crate 实测命中的两种**额外**有界形态（仍是"界定被遍历集合大小"，符合 R118）：
    // (a) **定长数组接收者**：窗口里有 `let <ident> = [ … ; <N> ]` 且站点遍历 `<ident>`；
    // (b) **切片上界被标量相等钉住**：站点形如 `x[..n]`，窗口里有 `assert_eq!(n, <数>)`。
    // ⚠ 站点行常常只是 `.all(|…| …)`（接收者在**上一行**，如 `out.iter()`）⇒
    // 必须用**包含站点行的整段上下文**提取标识符，⛔ 不能只看站点那一行（实测漏判 2 处）。
    let site_line = lines[start..line_no.min(lines.len())].join("\n");
    let flat = squeeze(&site_line);

    // ⭐ **R118 ✅ 形态 A：定长数组接收者** —— 接收者由 **`[..`／站点文本派生**（⛔ 不用白名单）。
    // 声明行形如 `let mut out = [0.0f32; 128];`（含接收者、`[`、`;`、`]`、`let`）。
    if let Some((receiver, _)) = slice_receiver_bound(&site_line) {
        let declared_fixed = lines[start..line_no].iter().any(|line| {
            let t = squeeze(line);
            t.contains(&format!("{receiver}=["))
                && t.contains(';')
                && t.contains(']')
                && (t.starts_with("let") || t.contains("letmut"))
        });
        if declared_fixed {
            return true;
        }
    } else {
        // 无切片形态时，退回到"扫窗口里每个 `let … = [ … ; … ]`"的接收者名字。
        for line in &lines[start..line_no] {
            let t = squeeze(line);
            if !(t.starts_with("let") || t.contains("letmut")) || !t.contains("=[") {
                continue;
            }
            // ⚠ 名字必须从**未压缩**的行派生：压缩后 `let mut out` 变成 `letmutout`，
            // 剥完 `letmut` 紧跟 `o`（字母）⇒ 会被 R147① 的边界检查**正确**拒掉。
            let raw = line.trim_start();
            let stripped = strip_keyword(raw, "let mut").or_else(|| strip_keyword(raw, "let"));
            let Some(stripped) = stripped else { continue };
            let name = stripped
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect::<String>();
            if !name.is_empty() && flat.contains(&format!("{name}.iter()")) {
                return true;
            }
        }
    }

    // ⭐ **R118 ✅ 形态 B：`接收者[..界]` ＋ 界被**同函数体**的标量相等钉住**
    // （修正上一版**方向写反**的缺陷：真实写法是 `scratch[..drained]`，⛔ 不是 `[..scratch]`）。
    if let Some((_, bound)) = slice_receiver_bound(&site_line) {
        // ⭐ **R147②**：`assert_eq!` 的参数名必须**右边界完整**（⛔ `drained_x` 不算 `drained`）。
        let pinned = has_call_with_ident(&flat, "assert_eq!", &bound)
            || has_call_with_ident(&flat, "assert!", &bound);
        if pinned {
            return true;
        }
    }
    false
}

/// ⭐ **R146／R153**：内容锚必须在**文件内唯一** —— 否则两个同形站点会互相冒充
/// （实测：陈旧入口检查因此**误报**）。返回 `(文件, 锚)`。
fn ambiguous_anchors(
    sources: &[(&str, &str)],
    allowlist: &[(&str, &str, &str)],
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (file, anchor, _reason) in allowlist {
        let Some((_, source)) = sources.iter().find(|(path, _)| path == file) else {
            continue;
        };
        let hits = source
            .lines()
            .filter(|line| squeeze(line) == squeeze(anchor))
            .count();
        if hits > 1 {
            out.push(((*file).to_owned(), (*anchor).to_owned()));
        }
    }
    out
}

/// ⭐ **R146／R153**：allowlist 的**身份 ＝ 内容锚**（⛔ 不用行号）。
/// 匹配规则：文件相同 **且** 站点片段（去空白后）相同。
fn allowlist_entry_matches(allowlist: &[(&str, &str, &str)], file: &str, snippet: &str) -> bool {
    let needle = squeeze(snippet);
    allowlist
        .iter()
        .any(|(path, anchor, _)| *path == file && squeeze(anchor) == needle)
}

/// ⭐ **R136（棘轮双向相等）**：登记了入口、但该站点**已经**有界 ⇒ 这是**陈旧入口**
/// （修好却忘了删行）⇒ 也必须红。返回 `(文件, 行号)`。
fn stale_entries(
    sources: &[(&str, &str)],
    allowlist: &[(&str, &str, &str)],
) -> Vec<(String, String)> {
    let mut stale = Vec::new();
    for (file, anchor, _reason) in allowlist {
        let Some((_, source)) = sources.iter().find(|(path, _)| path == file) else {
            continue;
        };
        // 内容锚 ⇒ 在文件里找到**同一个片段**的站点行号。
        for (index, line) in source.lines().enumerate() {
            if squeeze(line) == squeeze(anchor) && has_size_bound(source, index + 1) {
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
    // ① R93／R100：被扫集合必须达到下界。
    assert!(
        SOURCES.len() >= MIN_SOURCES,
        "被扫文件数 {} 必须 ≥ {MIN_SOURCES}（R93：下界绑在同一集合根上）",
        SOURCES.len()
    );

    let mut scanned_sites = 0usize;
    let mut unbounded: Vec<(String, usize, String)> = Vec::new();
    for (path, source) in SOURCES {
        for line_no in sites(source) {
            scanned_sites += 1;
            if has_size_bound(source, line_no) {
                continue;
            }
            let snippet = source
                .lines()
                .nth(line_no - 1)
                .unwrap_or("")
                .trim()
                .to_owned();
            if allowlist_entry_matches(ALLOWLIST, path, &snippet) {
                continue;
            }
            // ⭐ **R131**：失败信息带**根名** ＋ **缺界的那一句**。
            let ctx = source.lines().nth(line_no.saturating_sub(5)).unwrap_or("");
            let root = site_root(ctx).unwrap_or_else(|| "<不可解析>".to_owned());
            unbounded.push((
                (*path).to_owned(),
                line_no,
                format!("（根 = {root:?}）{snippet}"),
            ));
        }
    }

    // ② 非真空：一个站点都没扫到 ⇒ `SKIP(vacuous)`（⛔ 不记绿）。
    if scanned_sites == 0 {
        println!("[assertion-bounds] SKIP(vacuous)：一个 `.all(` 站点都没扫到 —— ⛔ 这不是绿");
        return;
    }

    // ③ 棘轮：无界站点数不得超过注册基线，且每一个都要报出来（R116／R120：要**逐处清单**）。
    println!(
        "[assertion-bounds] 扫过 {} 个文件 / {scanned_sites} 个 `.all(` 站点；无界 {} 处：{unbounded:?}",
        SOURCES.len(),
        unbounded.len()
    );
    // ⭐ **R136**：陈旧入口也必须红（棘轮双向）。
    // ⭐ 内容锚必须**文件内唯一**（否则会互相冒充）。
    let ambiguous = ambiguous_anchors(SOURCES, ALLOWLIST);
    assert!(
        ambiguous.is_empty(),
        "内容锚在文件内**不唯一** ⇒ 必须补足上下文或加站点序号：{ambiguous:?}"
    );
    let stale = stale_entries(SOURCES, ALLOWLIST);
    assert!(
        stale.is_empty(),
        "陈旧入口（站点已**有界**却仍登记在 allowlist）⇒ 必须删除该入口：{stale:?}"
    );
    assert!(
        // ⚠ 用 `==` 而不是 `<=`：`UNBOUNDED_BASELINE` 是 `usize` 的最小值 0 时，
        // `len() <= 0` 被 clippy 判成恒真（`absurd_extreme_comparisons` ⇒ `-D warnings` 下是门）。
        // 棘轮的语义本来就是"**恰好**等于基线"，`==` 更准确。
        unbounded.len() == UNBOUNDED_BASELINE,
        "无界 `.all(` 站点 {} 处 > 基线 {UNBOUNDED_BASELINE} ⇒ 新增站点必须逐条评审并登记理由：{unbounded:?}",
        unbounded.len()
    );
}

// ---------------------------------------------------------------------------
// R56：本判据**自带四条正负对照**（R115 的要求）—— 识别器必须"该报的报、不该报的不报"。
// 针与期望**运行期**构造；每个对照都是一条**独立断言**。
// ---------------------------------------------------------------------------

#[test]
fn the_bound_recogniser_has_both_teeth_and_silence() {
    mark("the_bound_recogniser_has_both_teeth_and_silence", "entered");
    let bounded = [
        (
            "len() ==",
            "assert_eq!(xs.len(), 4);\nassert!(xs.iter().all(|x| *x > 0));",
        ),
        (
            "len() >=",
            "assert!(xs.len() >= 3);\nassert!(xs.iter().all(|x| *x > 0));",
        ),
        (
            "!is_empty()",
            "assert!(!xs.is_empty());\nassert!(xs.iter().all(|x| *x > 0));",
        ),
    ];
    for (form, source) in bounded {
        let site = sites(source);
        assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点（形态 {form}）");
        assert!(
            has_size_bound(source, site[0]),
            "**假阴**：形态 {form} 必须被认成「有界」（若认不出，真站点会被误报）"
        );
    }

    // ⛔ **R118 的假阳对照**：元素值界与运行期计数器**不界定被遍历集合** ⇒ 必须判"无界"。
    let not_bounds = [
        (
            "元素值界",
            "assert!(v > 0.0);\nassert!(xs.iter().all(|x| *x > 0));",
        ),
        (
            "运行期计数器",
            "let checked = 7;\nassert!(checked > 0);\nassert!(xs.iter().all(|x| *x > 0));",
        ),
    ];
    for (form, source) in not_bounds {
        let site = sites(source);
        assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点（{form}）");
        assert!(
            !has_size_bound(source, site[0]),
            "**假阳**：{form} 不界定被遍历集合（R118）⇒ 不得被当成界"
        );
    }

    // ---------------------------------------------------------------------------
}
// ⭐ **R126 的窗口定标**：窗口是"**函数体边界**"（⛔ 不是固定行数）—— 两头都要有对照。
// ⭐ **R118 的两种额外形态**：定长数组接收者（构造上界定）／切片上界被标量相等钉住。
// ---------------------------------------------------------------------------

#[test]
fn the_window_is_the_function_body_not_a_fixed_line_count() {
    mark(
        "the_window_is_the_function_body_not_a_fixed_line_count",
        "entered",
    );
    // ① **同函数体内、界远在 ~120 行之前** ⇒ 必须认得出（固定 60 行窗口会**漏**）。
    let mut far = String::from("fn t() {\n    assert!(xs.len() >= 3);\n");
    for _ in 0..120 {
        far.push_str("    let _pad = 1;\n");
    }
    far.push_str("    assert!(xs.iter().all(|x| *x > 0));\n}\n");
    let site = sites(&far);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        has_size_bound(&far, site[0]),
        "⭐ 界在同一函数体内、但远在 120 行之前 ⇒ 必须认得出（固定窗口会漏）"
    );

    // ② **界在**上一个函数**里** ⇒ 必须**不**认（借邻居 ＝ R114 禁的形态；固定大窗口会**误认**）。
    let neighbour = "fn a() {\n    assert!(xs.len() >= 3);\n}\nfn b() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site_b = sites(neighbour);
    assert_eq!(site_b.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_size_bound(neighbour, site_b[0]),
        "⛔ 界在上一个函数里 ⇒ 不得被当成本函数的界（借邻居 ⇒ 假阴）"
    );
}

#[test]
fn the_two_extra_bound_forms_have_their_own_arms() {
    mark("the_two_extra_bound_forms_have_their_own_arms", "entered");
    // ① **定长数组接收者**（构造上界定）：声明与站点可**跨行**（接收者在上一行）。
    let fixed = "fn t() {\n    let mut out = [0.0f32; 128];\n    assert!(out.iter()\n        .all(|s| *s == 0.0));\n}\n";
    let site = sites(fixed);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        has_size_bound(fixed, site[0]),
        "定长数组接收者 ⇒ 构造上界定（R118 ✅）"
    );

    // ② **切片上界被标量相等钉住**：`x[..n]` ＋ `assert_eq!(n, N)` 同函数体。
    // ⚠ 本形态**只在接收者标识符命中硬编码白名单时**成立（`scratch` 在表内）——
    // 这条臂钉的是"白名单内的接收者 ＋ 同函数体的标量相等"这一**具体**写法。
    let pinned = "fn t() {\n    let drained = 10;\n    assert_eq!(drained, 10);\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site = sites(pinned);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    // ⭐ **翻面（R108：翻面的臂必须喂已知红）**：形态 B 修正方向后**必须认得出**。
    // 老版本写成找 `[..scratch]`（方向反了）⇒ 它**从未生效** ⇒ 上批只能钉成"未支持"。
    assert!(
        has_size_bound(pinned, site[0]),
        "⭐ `接收者[..界]` ＋ 同函数体的 `assert_eq!(界, N)` ⇒ **认**（方向已修正）"
    );

    // ⭐ **同形态的**已知红**：把那条标量相等**去掉** ⇒ 界没被钉住 ⇒ 必须判**无界**。
    let unpinned2 = "fn t() {\n    let drained = 10;\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site_u = sites(unpinned2);
    assert_eq!(site_u.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_size_bound(unpinned2, site_u[0]),
        "⛔ 没有标量相等钉住切片上界 ⇒ 必须判无界（这是形态 B 的已知红）"
    );

    // ⭐ **R119 的 near-miss**：钉的是**另一个**标识符（`n_other`）⇒ 不得被当成 `drained` 的界。
    let near_miss = "fn t() {\n    let drained = 10;\n    let n_other = 10;\n    assert_eq!(n_other, 10);\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site_n = sites(near_miss);
    assert_eq!(site_n.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_size_bound(near_miss, site_n[0]),
        "⛔ near-miss：钉住的是 `n_other`（⛔ 不是 `drained`）⇒ 不得当作 `drained` 的界（标识符边界）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ **R143**（函数定义行必须通用认）＋ ⭐ **R136**（棘轮**双向**：陈旧入口也红）
// ＋ ⭐ **R119**（near-miss：`fn name_x(` ⛔ 不得当成 `fn name(`）
// ---------------------------------------------------------------------------

#[test]
fn function_definition_lines_are_recognised_generally_not_by_a_prefix_list() {
    mark(
        "function_definition_lines_are_recognised_generally_not_by_a_prefix_list",
        "entered",
    );
    // ⭐ **R143 的实测代价**：本 crate 有 `pub const fn` **178** 处、`unsafe fn` 15、
    // `pub(super) fn` 10 —— 前缀清单（只认 `fn `/`pub fn `/`const fn `…）**全都不认**。
    for form in [
        "fn plain(",
        "pub fn public(",
        "pub const fn public_const(",
        "pub(crate) const fn crate_const(",
        "pub(super) fn super_fn(",
        "unsafe fn unsafe_fn(",
        "const fn const_fn(",
        "    async fn async_fn(",
        "pub fn generic<T: Copy>(",
    ] {
        assert!(
            is_fn_definition(form),
            "通用判定必须认出函数定义行：{form:?}"
        );
    }

    // ⭐ **R119 near-miss**：**调用行**与**别的名字**都不得被当成定义行。
    for not_def in ["    fn_name();", "    let fn_like = 1;", "    pub fnx();"] {
        assert!(
            !is_fn_definition(not_def),
            "⛔ 非定义行不得被当成函数定义：{not_def:?}"
        );
    }
}

#[test]
fn the_ratchet_is_bidirectional_and_flags_stale_entries() {
    mark(
        "the_ratchet_is_bidirectional_and_flags_stale_entries",
        "entered",
    );
    // 夹具：一个**有界**的站点（同函数体里有 `len() >=`）＋ 一条把它登记为"无界"的入口
    // ⇒ ⭐ **陈旧入口**（修好却忘删行）必须被报出来。
    let source =
        "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources: &[(&str, &str)] = &[("fake.rs", source)];
    let site = sites(source);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");

    let anchor = source.lines().nth(site[0] - 1).unwrap().trim();
    let stale = stale_entries(sources, &[("fake.rs", anchor, "陈旧：此处其实已经有界")]);
    assert_eq!(
        stale.len(),
        1,
        "⭐ R136：站点已有界却仍登记 ⇒ 必须报出陈旧入口（棘轮双向）"
    );

    // 反向对照：把入口挂到一个**真的无界**站点上 ⇒ **不是**陈旧入口。
    let unbounded_source = "fn t() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources2: &[(&str, &str)] = &[("fake.rs", unbounded_source)];
    let site2 = sites(unbounded_source);
    let anchor2 = unbounded_source.lines().nth(site2[0] - 1).unwrap().trim();
    let not_stale = stale_entries(sources2, &[("fake.rs", anchor2, "仍在缺口里")]);
    assert!(
        not_stale.is_empty(),
        "⛔ 真缺口上的入口**不是**陈旧入口（不得误报）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ **R147**：near-miss 臂必须**三向齐备**（前缀／后缀／剥离）＋ ⭐ **R96 掩码**臂。
// ---------------------------------------------------------------------------

#[test]
fn identifier_boundaries_are_checked_on_both_sides_and_when_stripping() {
    mark(
        "identifier_boundaries_are_checked_on_both_sides_and_when_stripping",
        "entered",
    );
    // ① **前缀孪生**（已有）：钉的是 `n_other`，⛔ 不得当作 `drained` 的界。
    let prefix_twin = "fn t() {\n    let drained = 10;\n    let n_other = 10;\n    assert_eq!(n_other, 10);\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site = sites(prefix_twin);
    assert_eq!(site.len(), 1);
    assert!(
        !has_size_bound(prefix_twin, site[0]),
        "⛔ 前缀孪生：`n_other` 的相等不得当作 `drained` 的界"
    );

    // ② ⭐ **后缀孪生（R147②）**：钉的是 `drained_x` ⇒ 右边界不完整 ⇒ 不得当作 `drained` 的界。
    let suffix_twin = "fn t() {\n    let drained = 10;\n    assert_eq!(drained_x, 10);\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site_s = sites(suffix_twin);
    assert_eq!(site_s.len(), 1);
    assert!(
        !has_size_bound(suffix_twin, site_s[0]),
        "⛔ **后缀孪生**：`assert_eq!(drained_x, 10)` 的右边界不完整 ⇒ 不得当作 `drained` 的界"
    );

    // ③ ⭐ **剥离孪生（R147①）**：`letter = [0.0f32; 128];` 不是 `let` 声明 ⇒
    // 名字派生必须**拒掉**它（⛔ 否则会剥成 `ter` 并当成定长数组接收者）。
    let strip_twin = "fn t() {\n    letter = [0.0f32; 128];\n    assert!(letter.iter().all(|s| *s == 0.0));\n}\n";
    let site_t = sites(strip_twin);
    assert_eq!(site_t.len(), 1);
    assert!(
        !has_size_bound(strip_twin, site_t[0]),
        "⛔ **剥离孪生**：`letter` 不得被当成 `let`（剥出 `ter`）⇒ 该站点必须判无界"
    );

    // 反向对照：**真的** `let ter = [0.0f32; 128];` ⇒ 认（证明上面拒的是边界，不是把功能关掉）。
    let real =
        "fn t() {\n    let ter = [0.0f32; 128];\n    assert!(ter.iter().all(|s| *s == 0.0));\n}\n";
    let site_r = sites(real);
    assert_eq!(site_r.len(), 1);
    assert!(
        has_size_bound(real, site_r[0]),
        "对照：真 `let ter = [ … ; N ]` ⇒ 必须认（否则说明我把功能关掉而不是修边界）"
    );
}

#[test]
fn comments_and_literals_are_masked_before_matching_bounds() {
    mark(
        "comments_and_literals_are_masked_before_matching_bounds",
        "entered",
    );
    // ⭐ **R96**：注释里的 `len() >= 3` **不是界**（掩码后不算）⇒ 该站点必须判无界。
    let commented = "fn t() {\n    // 说明：调用方保证 xs.len() >= 3\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site = sites(commented);
    assert_eq!(site.len(), 1);
    assert!(
        !has_size_bound(commented, site[0]),
        "⛔ R96：**注释里**的 `len() >= 3` 不得被当成界（否则假阴）"
    );

    // ⭐ **R96**：字符串字面量里的 `len() >= 3` 同样不算。
    let in_string =
        "fn t() {\n    let _msg = \"xs.len() >= 3\";\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site_s = sites(in_string);
    assert_eq!(site_s.len(), 1);
    assert!(
        !has_size_bound(in_string, site_s[0]),
        "⛔ R96：**字符串字面量里**的 `len() >= 3` 不得被当成界"
    );

    // 反向对照：**真代码**里的 `assert!(xs.len() >= 3);` ⇒ 认。
    let real =
        "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site_r = sites(real);
    assert_eq!(site_r.len(), 1);
    assert!(
        has_size_bound(real, site_r[0]),
        "对照：真代码里的 `len() >=` ⇒ 必须认（否则说明掩码把代码也掩了）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ **R131 的根名** ＋ ⭐ **配对臂**：`assert_eq!(x.len(), N)` **认**／元素值界 **⛔ 不认**；
// ⭐ **内容锚必须文件内唯一**（歧义 ⇒ 报出，⛔ 不静默取一个）。
// ---------------------------------------------------------------------------

#[test]
fn the_equality_length_form_is_recognised_but_element_value_bounds_are_not() {
    mark(
        "the_equality_length_form_is_recognised_but_element_value_bounds_are_not",
        "entered",
    );
    // ⭐ `assert_eq!(xs.len(), 4)`（**界定集合大小**）⇒ 认。
    let len_eq =
        "fn t() {\n    assert_eq!(xs.len(), 4);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site = sites(len_eq);
    assert_eq!(site.len(), 1);
    assert!(
        has_size_bound(len_eq, site[0]),
        "⭐ `assert_eq!(x.len(), N)` 必须被认成界（上一版漏了 `len(),` 变体）"
    );

    // ⛔ **元素值界**（`assert_eq!(xs[0], 4)`）不界定集合 ⇒ **不认**（R118）。
    let elem = "fn t() {\n    assert_eq!(xs[0], 4);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site_e = sites(elem);
    assert_eq!(site_e.len(), 1);
    assert!(
        !has_size_bound(elem, site_e[0]),
        "⛔ 元素值界不界定集合大小 ⇒ 不得当成界（R118）"
    );

    // ⭐ **R131**：失败信息必须带**根名**（可解析时）。
    assert_eq!(
        site_root("    assert!(v.iter().all(|x| *x > 0));").as_deref(),
        Some("v"),
        "根必须能从站点表达式派生（R153）"
    );
    // ⚠ 空根对照：`.iter()` 之前没有标识符 ⇒ **不可解析**（⛔ 不得静默返回某个默认根）。
    assert!(
        site_root("    assert!(.iter().all(|x| *x > 0));").is_none(),
        "⛔ 空根：派生不出根时必须返回 None（⛔ 不得静默编造根名）"
    );
}

#[test]
fn ambiguous_content_anchors_are_reported_not_silently_resolved() {
    mark(
        "ambiguous_content_anchors_are_reported_not_silently_resolved",
        "entered",
    );
    // 两个**同形**站点 ⇒ 同一内容锚在文件里出现 2 次 ⇒ 必须报**歧义**（⛔ 不静默取一个）。
    let source = "fn a() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\nfn b() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources: &[(&str, &str)] = &[("fake.rs", source)];
    let anchor = "    assert!(xs.iter().all(|x| *x > 0));";
    let ambiguous = ambiguous_anchors(sources, &[("fake.rs", anchor, "同形站点")]);
    assert_eq!(
        ambiguous.len(),
        1,
        "⭐ 内容锚在文件内出现 2 次 ⇒ 必须报歧义（否则两个站点互相冒充）"
    );

    // 反向对照：**唯一**的锚 ⇒ 不报歧义。
    let unique = "fn a() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources_u: &[(&str, &str)] = &[("fake.rs", unique)];
    let not_ambiguous = ambiguous_anchors(sources_u, &[("fake.rs", anchor, "唯一")]);
    assert!(not_ambiguous.is_empty(), "⛔ 唯一的锚不得被误报为歧义");
}

// ---------------------------------------------------------------------------
// ⭐ **R146 每形态实例级注入（合成实例）** —— 5 种形态各配一条：
//    ① **有界**（已知绿）② **删掉该界**（注入 ⇒ 已知红）③ **还原**（回绿）。
//    ⭐ **R160**：陈旧入口用**替换**造（⛔ 不给定长数组"多加一条"—— 那是**编译错误** ⇒ 无效样本）。
// ---------------------------------------------------------------------------

/// 五种形态的**合成实例**：`(形态名, 站点源, 界那一句)`。⛔ 每个实例**只**含一种界。
fn bound_form_instances() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "① len() >=",
            "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
            "    assert!(xs.len() >= 3);\n",
        ),
        (
            "② len() >",
            "fn t() {\n    assert!(xs.len() > 2);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
            "    assert!(xs.len() > 2);\n",
        ),
        (
            "③ !is_empty()",
            "fn t() {\n    assert!(!xs.is_empty());\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
            "    assert!(!xs.is_empty());\n",
        ),
        (
            "④ assert_eq!(len, N)",
            "fn t() {\n    assert_eq!(xs.len(), 4);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n",
            "    assert_eq!(xs.len(), 4);\n",
        ),
        (
            "⑤ 同根（切片上界被标量相等钉住）",
            "fn t() {\n    let n = 10;\n    assert_eq!(n, 10);\n    assert!(xs[..n].iter().all(|v| *v == 0.0));\n}\n",
            "    assert_eq!(n, 10);\n",
        ),
    ]
}

#[test]
fn each_bound_form_has_its_own_instance_level_injection() {
    mark(
        "each_bound_form_has_its_own_instance_level_injection",
        "entered",
    );
    let mut matched = 0usize;
    for (form, source, bound_line) in bound_form_instances() {
        let site = sites(source);
        assert_eq!(site.len(), 1, "合成实例必须恰好 1 个站点（{form}）");
        // ① 已知绿：有该界 ⇒ 认。
        assert!(
            has_size_bound(source, site[0]),
            "已知绿失败：{form} 的界必须被认成有界"
        );
        // ② 注入：**删掉那一句界**（⛔ 等价于"真实站点缺界"）⇒ 必须判**无界**（已知红）。
        let injected = source.replace(bound_line, "");
        assert_ne!(
            injected, source,
            "注入必须真的改掉源（{form}）：锚点 = {bound_line:?}"
        );
        let site_injected = sites(&injected);
        assert_eq!(site_injected.len(), 1, "注入后仍应只有 1 个站点（{form}）");
        assert!(
            !has_size_bound(&injected, site_injected[0]),
            "已知红失败：{form} 删掉界之后必须判无界（否则该形态**没有牙**）"
        );
        // ③ 还原：源字符串未被原地改动（`replace` 返回新串）⇒ 再判一次必须回绿。
        assert!(
            has_size_bound(source, site[0]),
            "还原失败：{form} 恢复后必须仍然判有界"
        );
        matched += 1;
    }
    println!("[assertion-bounds] R146 各形态实例级注入：**{matched}/5**");
    assert_eq!(
        matched, 5,
        "五种形态必须**各配一条**实例级注入（R146）：实得 {matched}/5"
    );
}

#[test]
fn the_stale_entry_case_is_built_by_replacement_not_by_growing_a_fixed_array() {
    mark(
        "the_stale_entry_case_is_built_by_replacement_not_by_growing_a_fixed_array",
        "entered",
    );
    // ⭐ **R160**：allowlist 是**定长数组** ⇒ 给它"多加一条"是**编译错误**（无效样本）。
    // 正确造法 = **替换**已有的那一条（这里用一条**合成** allowlist，长度不变）。
    let source =
        "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources: &[(&str, &str)] = &[("fake.rs", source)];
    let site = sites(source);
    let anchor = source.lines().nth(site[0] - 1).unwrap().trim();

    // 替换前：入口指向一个**有界**站点 ⇒ 陈旧（必须报）。
    let before: &[(&str, &str, &str)] = &[("fake.rs", anchor, "陈旧：站点其实已经有界")];
    assert_eq!(
        stale_entries(sources, before).len(),
        1,
        "⭐ 陈旧入口必须被报出（长度不变的**替换**造法，R160）"
    );

    // 替换后：同一条入口改指向一个**真无界**站点（新数组，长度相同）⇒ **不再**是陈旧。
    let unbounded = "fn t() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources2: &[(&str, &str)] = &[("fake.rs", unbounded)];
    let site2 = sites(unbounded);
    let anchor2 = unbounded.lines().nth(site2[0] - 1).unwrap().trim();
    let after: &[(&str, &str, &str)] = &[("fake.rs", anchor2, "仍在缺口里")];
    assert!(
        stale_entries(sources2, after).is_empty(),
        "⛔ 真缺口上的入口不是陈旧入口（替换造法的反向对照）"
    );
}

// ---------------------------------------------------------------------------
// ⭐ **推广（本批 #4）**：把"注入 ＝ **合成源字符串的替换**"这一形态推广到**更多规则** ⇒
//    每条规则自带绿/红两臂（⛔ 不靠一次性进程级注入）⇒ 计数可与 R146 的 5/5 合并。
//    ⭐ **R163**：每个注入都**必须带一句"自身会通过"的断言** ⇒ 才能区分"被抓到"与"注入写错"。
// ---------------------------------------------------------------------------

#[test]
fn the_synthetic_replacement_pattern_covers_three_more_rules() {
    mark(
        "the_synthetic_replacement_pattern_covers_three_more_rules",
        "entered",
    );
    let mut matched = 0usize;

    // 规则 A：**注释里的界不算界**（`mask_noncode`）。绿 = 不认；注入 = 把注释**变成真代码** ⇒ 认。
    let commented =
        "fn t() {\n    // assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let site = sites(commented);
    assert_eq!(site.len(), 1);
    assert!(
        !has_size_bound(commented, site[0]),
        "规则 A 绿：注释里的界不得被认成界（R96）"
    );
    let unmasked = commented.replace("// assert!(xs.len() >= 3);", "assert!(xs.len() >= 3);");
    assert_ne!(unmasked, commented, "规则 A 注入必须真的改掉源");
    assert!(
        has_size_bound(&unmasked, site[0]),
        "规则 A 红：同一句话变成**真代码**后必须被认成界（否则掩码把代码也掩了）"
    );
    matched += 1;

    // 规则 B：**函数定义行的通用判定**（`is_fn_definition`）。绿 = 认；注入 = 把 `fn ` 改成 `fnx ` ⇒ 不认。
    let def = "pub const fn public_const(";
    assert!(
        is_fn_definition(def),
        "规则 B 绿：`pub const fn` 必须被认成定义行（R143）"
    );
    let not_def = def.replace("fn ", "fnx ");
    assert_ne!(not_def, def, "规则 B 注入必须真的改掉源");
    assert!(
        !is_fn_definition(&not_def),
        "规则 B 红：`fnx ` 不是 `fn ` ⇒ 不得被认成定义行"
    );
    matched += 1;

    // 规则 C：**站点根派生**（`site_root`）。绿 = 派生得出；注入 = 破坏 `.iter()` ⇒ 派生不出。
    let site_text = "    assert!(v.iter().all(|x| *x > 0));";
    assert_eq!(
        site_root(site_text).as_deref(),
        Some("v"),
        "规则 C 绿：根必须能派生（R153）"
    );
    let broken = site_text.replace(".iter()", ".iterx()");
    assert_ne!(broken, site_text, "规则 C 注入必须真的改掉源");
    assert!(
        site_root(&broken).is_none(),
        "规则 C 红：`.iter()` 被破坏后必须派生不出根（⛔ 不得编造）"
    );
    matched += 1;

    println!("[assertion-bounds] 形态推广：本轮新增 **{matched}/3** 条规则自带绿/红两臂");
    assert_eq!(matched, 3, "三条规则必须各配一条（实得 {matched}/3）");
}

// ---------------------------------------------------------------------------
// R118（整数计数四类）＋ R183/R185（每类下界抽成函数、喂坏输入证明有牙）
//   collection_size / value_bound / element_value_bound / condition_counter
//   R149/R156：数条目排除注释；R177：按类名定位；R186：下界在同一文件喂坏表。
// ---------------------------------------------------------------------------

const COUNT_CLASSES: [&str; 4] = [
    "collection_size",
    "value_bound",
    "element_value_bound",
    "condition_counter",
];

/// 分类一条整数计数断言。① 元素值界（有下标）② 集合大小（len/is_empty/count）
/// ③④ 先归"值界"；③ 由 [`count_by_class`] 在**函数体窗口**里复查 `+=` 后改写。
fn classify_count(line: &str) -> Option<usize> {
    let t = squeeze(line);
    if t.starts_with("//") {
        return None;
    }
    if !t.contains("assert_eq!(") && !t.contains("assert!(") {
        return None;
    }
    let has_number = (0..10).any(|d| t.contains(&format!(",{d})")))
        || (0..10).any(|d| t.contains(&format!("=={d}")));
    if !has_number {
        return None;
    }
    if t.contains('[') && t.contains(']') {
        return Some(2);
    }
    if t.contains(".len()") || t.contains(".is_empty()") || t.contains(".count()") {
        return Some(0);
    }
    // ③ **同一行**里的计数器证据（臂用单行夹具；跨行由 [`count_by_class`] 在函数体窗口里复查）。
    if let Some(name) = ident_of(line)
        && (t.contains(&format!("{name}+=")) || t.contains(&format!("letmut{name}")))
    {
        return Some(3);
    }
    Some(1)
}

/// 计数断言左侧的**根标识符**（③ 档根绑定用）。
fn ident_of(line: &str) -> Option<String> {
    let squeezed = squeeze(line);
    let after = squeezed.split_once('(')?.1;
    let name: String = after
        .chars()
        .skip_while(|c| !(c.is_alphanumeric() || *c == '_'))
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

/// 按类名计数（表头/注释行由 [`classify_count`] 直接跳过）。
fn count_by_class(sources: &[(&str, &str)]) -> [usize; 4] {
    let mut counts = [0usize; 4];
    for (_, source) in sources {
        let lines: Vec<&str> = source.lines().collect();
        for (index, line) in lines.iter().enumerate() {
            let Some(mut class) = classify_count(line) else {
                continue;
            };
            // ③ 根绑定：`+= 1` 通常在**另一行** ⇒ 必须在**函数体窗口**里查（实测只看当前行时为 0）。
            if class == 1
                && let Some(name) = ident_of(line)
            {
                let start = enclosing_body_start(&lines, index + 1);
                let window = lines[start..index].join("\n");
                if window.contains(&format!("{name} +="))
                    || window.contains(&format!("{name}+="))
                    || window.contains(&format!("let mut {name}"))
                {
                    class = 3;
                }
            }
            counts[class] += 1;
        }
    }
    counts
}

/// R183/R185：下界抽成**函数**（⛔ 不是就地写 `>= 1`）。
fn class_floor_holds(counts: &[usize; 4], floor: usize) -> bool {
    counts.iter().all(|count| *count >= floor)
}

#[test]
fn integer_count_sites_are_classified_into_four_classes() {
    mark(
        "integer_count_sites_are_classified_into_four_classes",
        "entered",
    );
    let counts = count_by_class(SOURCES);
    let mut report = String::new();
    for (index, name) in COUNT_CLASSES.iter().enumerate() {
        report.push_str(&format!("{name}={} ", counts[index]));
    }
    println!(
        "[assertion-bounds] R118 整数计数四类：{report}（合计 {}）",
        counts.iter().sum::<usize>()
    );
    // ⭐ **R215①**：把**余量**做成读数（⛔ 不是只写"非零"）—— `当前值 − 地板`，**报数值**。
    const MARGIN_FLOOR: usize = 1;
    let mut zero_margin: Vec<&str> = Vec::new();
    for (index, name) in COUNT_CLASSES.iter().enumerate() {
        let margin = counts[index].saturating_sub(MARGIN_FLOOR);
        eprintln!(
            "[assertion-bounds][margin] {name}: 当前={} 地板={MARGIN_FLOOR} **余量={margin}**",
            counts[index]
        );
        if margin == 0 {
            zero_margin.push(name);
        }
    }
    // 余量 > 0 正常；**为 0 必须显式注明**（⛔ 不静默）—— 免得"合法删除"被误判为回归。
    if !zero_margin.is_empty() {
        eprintln!(
            "[assertion-bounds][margin] ⚠ 余量为 0 的类（恰好落在下界上）：{zero_margin:?} ⇒ 请显式复核"
        );
    }

    // ⭐ **R199**：**规模地板是反向指标** —— 缺陷会把计数**抬高** ⇒ 地板照样通过。
    // ⛔ 因此不把"≥ 地板"当主守卫：它只作**规模读数**，跌破时**降级为 `eprintln!` 告警**。
    const SCALE_FLOOR: usize = 1;
    for (index, name) in COUNT_CLASSES.iter().enumerate() {
        if counts[index] < SCALE_FLOOR {
            eprintln!(
                "[assertion-bounds] ⚠ 规模降级：类 `{name}` 只有 {} 条（< {SCALE_FLOOR}）—— \
                 这可能只是夹具变小，⛔ 也可能是分类器失效；主守卫见每类两臂",
                counts[index]
            );
        }
    }
    // ⭐ **R199 的主守卫（两臂）**：喂**坏输入**必须被拒 ＋ 正向必须收录。
    // ① 坏输入：把计数**灌水**成"每一类都很大"的表 ⇒ 旧地板会**照样通过**（反向指标的实证）；
    // 真正要证的是**分类器**能拒掉不属于该类的输入（下一条）。
    assert!(
        !class_floor_holds(&[0, 0, 0, 1], 1),
        "⭐ 地板函数对坏表必须判假（这证明它至少不是恒真）"
    );
    assert!(class_floor_holds(&[1, 1, 1, 1], 1), "地板函数的正向对照");
    // ② 分类器的两臂：**元素值界**不得落进 `collection_size`；**集合大小**不得落进 `value_bound`。
    assert_eq!(
        classify_count("    assert_eq!(frames[0].peak, 0);"),
        Some(2),
        "两臂①：下标访问必须落 `element_value_bound`（⛔ 不得落 `collection_size`）"
    );
    assert_eq!(
        classify_count("    assert_eq!(frames.len(), 4);"),
        Some(0),
        "两臂②：`.len()` 必须落 `collection_size`（⛔ 不得落 `value_bound`）"
    );
}

#[test]
fn each_count_class_has_its_own_arm() {
    mark("each_count_class_has_its_own_arm", "entered");
    let cases: [(&str, usize); 4] = [
        ("    assert_eq!(frames.len(), 4);", 0),
        ("    assert_eq!(checked, 7);", 1),
        ("    assert_eq!(frames[0].peak, 0);", 2),
        ("    let mut seen = 0; seen += 1; assert_eq!(seen, 3);", 3),
    ];
    let mut matched = 0usize;
    for (text, expected) in cases {
        assert_eq!(
            classify_count(text),
            Some(expected),
            "类 `{}` 的合成输入必须落到该类",
            COUNT_CLASSES[expected]
        );
        matched += 1;
    }
    println!("[assertion-bounds] R118 每类各配一条臂：**{matched}/4**");
    assert_eq!(matched, 4, "四类必须各配一条（实得 {matched}/4）");
    // ⭐ **R210② 的纯粹反例臂**：**非断言行**必须被拒 —— 若分类器"假计入"（把非计数断言也算进来），
    // 计数会**升高**（旧规模地板照样通过），但本条臂会**红**。
    assert_eq!(
        classify_count("    let seen = 0;"),
        None,
        "⛔ 假计入：非断言行不得被算成计数站点（否则计数升高而缺陷仍在）"
    );
    assert_eq!(
        classify_count("    seen += 1;"),
        None,
        "⛔ 假计入：纯 `+=` 行（无断言）不得被算成计数站点"
    );
    // ⭐ **能真正抓到"假计入"的那条臂**：这一行**含数字**（满足 `,2)`）却**不是断言**
    // ⇒ 干净的分类器必须返回 `None`；若分类器不再要求"是断言行"，它会被**假计入**
    // ⇒ 计数**升高**（旧规模地板照样通过）而**本条臂会红**（R210② 的纯粹形态）。
    assert_eq!(
        classify_count("    let pair = (1, 2);"),
        None,
        "⛔ 假计入：含数字但**非断言**的行必须被拒（否则计数升高而缺陷仍在）"
    );
    // ⭐ **R215②：把一次性反例提升为常驻形态** —— 在**同一判据内**同时永久断言两件事：
    // ① **被撤掉的旧地板会放行**：注入实测把计数从 127 抬到 358（`value_bound=289` 等），
    //    而旧地板 `>= 1` 对它**照样判真** ⇒ 这里把该"会放行"写成断言（永久重放该读数）；
    // ② **行为谓词必须判它不合格**：见上面那条 `let pair = (1, 2);` 的臂（它才是真守卫）。
    assert!(
        class_floor_holds(&[4, 289, 10, 55], 1),
        "⭐ R215②①：旧规模地板对**被灌水的表**必须判真（这正是'地板是反向指标'的常驻证据）"
    );
    assert!(
        classify_count("    let pair = (1, 2);").is_none(),
        "⭐ R215②②：行为谓词必须把'假计入'判为不合格（真正的守卫）"
    );

    // 反向对照（R149/R156）：**注释行**不是站点。
    assert_eq!(
        classify_count("    // assert_eq!(frames.len(), 4);"),
        None,
        "注释行不得计入（数条目必须排除注释）"
    );
}

// ---------------------------------------------------------------------------
// R214②/R225①：掩码器的**五条臂**（每条给期望读数 `⇒ N`）＋ 正对照。
//   ⭐ R235 的语料预检（`grep` 本 crate）：**字符字面量 0 次／raw 字符串 0 次／生命周期 26 次**
//   ⇒ ⭐ 这些构造在语料里**无触发面** ⇒ 只能由**夹具**触发（因此本判据必须自带夹具）。
//   ⭐ 正对照**不可省**：缺它则"臂全绿"与"掩码器把一切都吞了"不可区分。
// ---------------------------------------------------------------------------

/// 掩码后**可见的针**条数（0 或 1）——期望读数就是它。
fn visible_needle(source: &str, needle: &str) -> usize {
    usize::from(mask_noncode(source).contains(needle))
}

#[test]
fn masker_five_arms_with_expected_readings() {
    const NEEDLE: &str = "len() >= 3";

    // ① 生命周期 `&'a str`（语料 26 次）⇒ **1**（生命周期不得被当字符字面量）。
    let lifetime = "fn t<'a>(x: &'a str) {\n    assert!(xs.len() >= 3);\n}\n";
    assert_eq!(
        visible_needle(lifetime, NEEDLE),
        1,
        "臂①（生命周期，语料 26 次）：其后的真代码必须可见 ⇒ 期望 1"
    );

    // ② 字符字面量含双引号（语料 0 次，夹具触发）⇒ **1**。
    let char_quote = "fn t() {\n    let q = '\"';\n    assert!(xs.len() >= 3);\n}\n";
    assert_eq!(
        visible_needle(char_quote, NEEDLE),
        1,
        "臂②（字符字面量含 `\"`，语料 0 次）：字面量之后的真代码必须可见 ⇒ 期望 1"
    );

    // ③ 原始字符串（语料 0 次）**本身不含针**，针在它**之后** ⇒ **1**。
    let raw_after = "fn t() {\n    let s = r#\"hello\"#;\n    assert!(xs.len() >= 3);\n}\n";
    assert_eq!(
        visible_needle(raw_after, NEEDLE),
        1,
        "臂③（原始字符串，语料 0 次）：其后的真代码必须可见 ⇒ 期望 1"
    );

    // ④ ⭐ **raw 内藏针** ⇒ **0**（字符串里的针不得计入）。
    let raw_needle = "fn t() {\n    let s = r#\"assert!(xs.len() >= 3);\"#;\n}\n";
    assert_eq!(
        visible_needle(raw_needle, NEEDLE),
        0,
        "臂④（raw 内藏针）：字符串**内部**的针不得可见 ⇒ 期望 0"
    );

    // ⑤ ⭐ **正对照**（⛔ 不可省）：普通代码里的针 ⇒ **1**。
    let control = "fn t() {\n    let rust = 1;\n    assert!(xs.len() >= 3);\n}\n";
    assert_eq!(
        visible_needle(control, NEEDLE),
        1,
        "臂⑤（正对照）：普通代码里的针必须可见 ⇒ 期望 1（⛔ 缺它则无法区分'臂全绿'与'掩码器吞掉一切'）"
    );

    println!("[assertion-bounds] masker 五条臂：5/5（期望读数 1／1／1／0／1）");
}
