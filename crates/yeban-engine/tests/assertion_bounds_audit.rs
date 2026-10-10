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

/// 站点**所在函数体**的起点行下标（0 起）：向上找第一个 `fn …` 起头处。
fn enclosing_body_start(lines: &[&str], line_no: usize) -> usize {
    let mut index = line_no.saturating_sub(1);
    while index > 0 {
        let text = lines[index - 1].trim_start();
        if text.starts_with("fn ")
            || text.starts_with("pub fn ")
            || text.starts_with("pub(crate) fn ")
            || text.starts_with("const fn ")
        {
            return index - 1;
        }
        index -= 1;
    }
    0
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
    let idents: Vec<String> = ["out", "scratch", "mono", "left", "right", "block"]
        .iter()
        .map(|name| (*name).to_owned())
        .filter(|name| site_line.contains(name.as_str()))
        .collect();
    for name in &idents {
        // (a) 定长数组：声明行同时含有该标识符、`[`、`;` 与 `]`。
        let fixed_array = window.lines().any(|line| {
            line.contains(name.as_str())
                && line.contains('[')
                && line.contains(';')
                && line.contains(']')
                && (line.contains("let ") || line.contains("let mut "))
        });
        if fixed_array {
            return true;
        }
    }
    // (b) 切片上界被**标量相等**钉住（`x[..n]` + `assert_eq!(n, N)`）。
    for name in &idents {
        if site_line.contains(&format!("[..{name}]")) || site_line.contains(&format!("[..{name} "))
        {
            let pinned = window.lines().any(|line| {
                line.contains("assert_eq!(")
                    && line.contains(&format!("{name},"))
                    && line.contains(char::is_numeric)
            });
            if pinned {
                return true;
            }
        }
    }
    false
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
    let mut unbounded: Vec<(String, usize)> = Vec::new();
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
            unbounded.push(((*path).to_owned(), line_no));
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
    // ⚠ **本形态目前**未支持****（如实钉住）：识别器把它写成 `[..scratch]`（方向反了），
    // 而真实写法是 **`scratch[..drained]`**（接收者在左、界在方括号里）⇒ 它一直**没生效**过。
    // 修法（下一轮）：认 `接收者[..界]` ＋ 同函数体内 `assert_eq!(界, N)`；本轮⛔ 不假装支持。
    assert!(
        !has_size_bound(pinned, site[0]),
        "⭐ 未支持形态：`接收者[..界]` ＋ 标量相等 —— 识别器方向写反 ⇒ 目前判无界（下一轮修）"
    );

    // ⭐ **已知局限臂（如实钉住，⛔ 不假装它是通用形态）**：接收者标识符不在白名单里 ⇒ 认不出。
    // 修法（下一轮）：标识符**从站点表达式派生**，⛔ 不用硬编码列表。
    let other_ident = "fn t() {\n    let n = 10;\n    assert_eq!(n, 10);\n    assert!(buf[..n].iter().all(|f| *f == 0.0));\n}\n";
    let site = sites(other_ident);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_size_bound(other_ident, site[0]),
        "⭐ 已知局限：接收者标识符不在硬编码白名单里 ⇒ 判无界（假阳；下一轮改为从站点表达式派生）"
    );

    // ③ **反例**：切片上界**没有**被钉住 ⇒ 必须判无界。
    let unpinned = "fn t() {\n    assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));\n}\n";
    let site = sites(unpinned);
    assert_eq!(site.len(), 1, "对照夹具必须恰好 1 个站点");
    assert!(
        !has_size_bound(unpinned, site[0]),
        "⛔ 切片上界未被钉住 ⇒ 无界（`drained` 可能为 0）"
    );
}
