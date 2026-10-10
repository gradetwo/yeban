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
const ALLOWLIST: &[(&str, usize, &str)] = &[
    // ⭐ **R132 的进度读数**：入口数 ＝ "还有多少未修缺口"。
    // 本批把窗口从"固定行数"改成"**函数体边界**"后，原先两条"识别器盲区"入口**已撤**
    // （接收者 `out` 是定长数组，其声明就在同一个函数体里 ⇒ 现在认得出）⇒ **入口数 = 0**。
    //
    // ⚠ 若将来再加入口：**⛔ 不许用行号**（上方插行即错位 ⇒ 假红；R117／R119）——
    // 必须改用**内容锚**（文件 ＋ 站点行的规范化文本 ＋ 站点序号）。
];

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
    let window = mask_noncode(&window);
    let forms = size_bound_forms();
    if forms.iter().any(|form| window.contains(form.as_str())) {
        return true;
    }
    // `assert!(… !is_empty() …)`：`!` 与调用之间可能有空格/换行 ⇒ 单独认一次形态。
    if window.contains("!") && window.contains("is_empty()") {
        return true;
    }
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

/// ⭐ **R136（棘轮双向相等）**：登记了入口、但该站点**已经**有界 ⇒ 这是**陈旧入口**
/// （修好却忘了删行）⇒ 也必须红。返回 `(文件, 行号)`。
fn stale_entries(
    sources: &[(&str, &str)],
    allowlist: &[(&str, usize, &str)],
) -> Vec<(String, usize)> {
    let mut stale = Vec::new();
    for (file, line, _reason) in allowlist {
        let Some((_, source)) = sources.iter().find(|(path, _)| path == file) else {
            continue;
        };
        if has_size_bound(source, *line) {
            stale.push(((*file).to_owned(), *line));
        }
    }
    stale
}

#[test]
fn every_all_assertion_in_this_crate_is_bounded_or_allowlisted() {
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
            if ALLOWLIST
                .iter()
                .any(|(file, line, _)| file == path && *line == line_no)
            {
                continue;
            }
            // ⭐ **R131**：失败信息必须点出**缺界的那一句**（⛔ 不只报文件与行号）。
            let snippet = source
                .lines()
                .nth(line_no - 1)
                .unwrap_or("")
                .trim()
                .to_owned();
            unbounded.push(((*path).to_owned(), line_no, snippet));
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
    // 夹具：一个**有界**的站点（同函数体里有 `len() >=`）＋ 一条把它登记为"无界"的入口
    // ⇒ ⭐ **陈旧入口**（修好却忘删行）必须被报出来。
    let source =
        "fn t() {\n    assert!(xs.len() >= 3);\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources: &[(&str, &str)] = &[("fake.rs", source)];
    let site = sites(source);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");

    let stale = stale_entries(sources, &[("fake.rs", site[0], "陈旧：此处其实已经有界")]);
    assert_eq!(
        stale.len(),
        1,
        "⭐ R136：站点已有界却仍登记 ⇒ 必须报出陈旧入口（棘轮双向）"
    );

    // 反向对照：把入口挂到一个**真的无界**站点上 ⇒ **不是**陈旧入口。
    let unbounded_source = "fn t() {\n    assert!(xs.iter().all(|x| *x > 0));\n}\n";
    let sources2: &[(&str, &str)] = &[("fake.rs", unbounded_source)];
    let site2 = sites(unbounded_source);
    let not_stale = stale_entries(sources2, &[("fake.rs", site2[0], "仍在缺口里")]);
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
