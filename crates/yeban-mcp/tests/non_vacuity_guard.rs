//! **R93（本 crate 的发现）＋ R94 的源码级守卫**：遍历型判据必须断言"被扫集合非空且 ≥ 下界"，
//! 路径比较必须先规范化分隔符。
//!
//! ## 为什么需要这一份
//!
//! 第五批实测（已升为 **R93**）：普查里的 `path.contains("/src/")` **在 Windows 上恒 false**
//! ⇒ **扫描面为空 ⇒ 判据恒绿**，而 Linux 腿永远看不见 —— 这是"空判据"家族的新形状：
//! ⛔ 不是断言写错，而是**被扫描的集合是空的**。
//!
//! 本文件把这条纪律**机械化**（而不是靠人记得）：
//!
//! | 判据 | 它挡住的形态 |
//! | :--- | :--- |
//! | [`dynamic_scans_declare_a_lower_bound`] | 用了 `read_rust_sources`/`read_dir`/`fs::read*` 的函数**没有任何非空/下界断言** |
//! | [`path_comparisons_are_normalized`] | `<path-ish>.ends_with("a/b")` 这类**跨平台会假失败或真空通过**的比较 |
//! | [`the_guard_registry_is_not_empty_and_each_needle_exists`] | 守卫**自己**退化（针不再出现在源码里 ⇒ 守卫恒绿） |
//!
//! ⚠ 与 R48 的分工：R48 管"枚举/表"，本条管"**被扫的集合**"。两者都是"空判据"家族的形状。
//! ⚠ 与 R94 的分工：R94 管"扫描器怎么掩码"，本条管"扫完有没有检查**真的扫到了东西**"。

use std::path::PathBuf;

use yeban_mcp::undo_session::read_rust_sources;

/// **动态集合源**（R93 的题面：目录遍历/枚举后断言性质）：能在运行期变成**空集**的东西。
///
/// ⚠ 口径：**单文件读取**（`fs::read_to_string(path)` / `fs::read(path)`）**不算** ——
/// 它不是"遍历一个集合后断言性质"，没有"空集合 ⇒ 恒绿"的形状。
/// ⚠ 只含动态源但**不做任何断言**的**取数助手**（`fn all_sources() -> Vec<...>`）也不算：
/// 下界的责任在**消费它的判据**身上，不在取数函数身上。
///
/// ⚠ **第八批实测**：`DYNAMIC_SOURCES` 与 `DEFENSIVE_SOURCES` 必须分开 ——
/// 本 crate **不使用** `WalkDir` / `glob(`（实测 0 处命中）⇒ 它们**不能**参与
/// "每个针都必须在别处存在"的断言（否则那条断言在检查**虚构**的针）。
/// 但它们**仍参与识别**（将来引入时不许漏检）。
const DYNAMIC_SOURCES: [&str; 2] = ["read_rust_sources", "read_dir"];
/// **防御性**识别的动态源：本 crate 目前不用，仍参与识别，⛔ 不参与存在性断言。
const DEFENSIVE_SOURCES: [&str; 2] = ["WalkDir", "glob("];

/// 识别用的**全部**动态源（必需 ＋ 防御性）。
fn all_dynamic_sources() -> impl Iterator<Item = &'static str> {
    DYNAMIC_SOURCES.into_iter().chain(DEFENSIVE_SOURCES)
}

/// **非真空形态**：命中任一即认为该函数声明了"被扫集合非空／达到下界"。
const NON_VACUITY_FORMS: [&str; 5] = ["is_empty()", ".len(), ", "non_empty", "scanned", "seen =="];

/// 路径比较里"看起来像路径"的接收者名（大小写不敏感的子串）。
const PATHISH: [&str; 6] = ["path", "dir", "file", "source", "root", "suffix"];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 全部 `.rs`（`src` ＋ `tests`），按路径排序（R93：排序保证读数与文件系统顺序无关）。
fn all_sources() -> Vec<(String, String)> {
    let root = manifest_dir();
    let mut files = read_rust_sources(&[root.join("src"), root.join("tests")]);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// 一个函数的 `(起始行号, 名字, 正文)`。
#[derive(Debug)]
struct Function {
    line: usize,
    name: String,
    body: String,
}

/// 极简函数切分：`fn <名>(` 起，到**同缩进**的第一个 `}` 行止。
///
/// 为什么够用：本 crate 的源码是 rustfmt 规范化的 ⇒ 嵌套块闭合的缩进**更深**，
/// 因此"同缩进 + `}`"唯一对应函数结尾。
fn functions(text: &str) -> Vec<Function> {
    let lines: Vec<&str> = text.lines().collect();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < lines.len() {
        let line = lines[index];
        let indent = line.len() - line.trim_start().len();
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("fn ") {
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            if !name.is_empty() {
                // 找同缩进的 `}`
                let closer = format!("{}}}", " ".repeat(indent));
                let mut end = index + 1;
                while end < lines.len() && lines[end] != closer {
                    end += 1;
                }
                let body = lines[index..end.min(lines.len())].join("\n");
                out.push(Function {
                    line: index + 1,
                    name,
                    body,
                });
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    out
}

/// `>= <数字>` / `> <数字>` 形态（非真空下界的常见写法）。
fn has_comparison_bound(body: &str) -> bool {
    for operator in [">= ", "> "] {
        let mut from = 0usize;
        while let Some(offset) = body[from..].find(operator) {
            let at = from + offset + operator.len();
            if body[at..].starts_with(|c: char| c.is_ascii_digit()) {
                return true;
            }
            from = at;
        }
    }
    false
}

fn declares_lower_bound(body: &str) -> bool {
    NON_VACUITY_FORMS.iter().any(|form| body.contains(form)) || has_comparison_bound(body)
}

/// 一行里的"路径型比较"。返回 `Some(接收者)` 表示它**需要**规范化证据。
fn unnormalized_path_comparison(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }
    for method in [".ends_with(\"", ".starts_with(\"", ".contains(\""] {
        let mut from = 0usize;
        while let Some(offset) = line[from..].find(method) {
            let at = from + offset;
            let literal_start = at + method.len();
            let Some(end) = line[literal_start..].find('"') else {
                break;
            };
            let literal = &line[literal_start..literal_start + end];
            let receiver: String = line[..at]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let lower = receiver.to_ascii_lowercase();
            let pathish = PATHISH.iter().any(|needle| lower.contains(needle));
            let has_separator = literal.contains('/') || literal.contains('\\');
            if pathish && has_separator {
                // 规范化证据：(a) 走共享助手；(b) 显式替换分隔符；(c) 同行给了**两种**分隔符的写法。
                let normalized = line.contains("path_ends_with(")
                    || line.contains("normalized_path(")
                    || line.contains("replace('\\\\'")
                    || line.contains("replace(\"\\\\\"")
                    // (c) 同行**显式处理两种分隔符**（`a/b` 与 `a\\b` 一起写）也算规范化。
                    || (line.contains('/') && line.contains('\\'));
                if !normalized {
                    return Some(receiver);
                }
            }
            from = literal_start;
        }
    }
    None
}

/// 同文件里"被绑定到**含分隔符**字面量"的标识符 ⇒ `(标识符, 字面量)`。
///
/// 这是新臂 [`unnormalized_variable_path_comparison`] **唯一**的额外前提：
/// 只有当某个标识符**确有**一个含 `/` 或 `\` 的字符串字面量绑定时，
/// `x.ends_with(那个标识符)` 才算"路径比较"。⛔ 没有这个前提就会把
/// 集合成员判定、源码文本包含、单分量常量一起误伤 —— 那四种形态的**逐字反例**
/// 见 `variable_path_comparisons_are_normalized` 的已知绿样本。
///
/// ## 口径（刻意很窄，每条都是普查实测过的取舍）
///
/// - 只认 `let` / `const` / `static`，**只认同行赋值**，允许 `mut` 与 `: 类型`；
/// - 字面量取"赋值号后第一个 `"` 到下一个 `"`"之间的文本（**不解析转义**）；
/// - ⛔ 不跨文件、⛔ 不跨函数、⛔ 不追参数传入 —— 那些形态本臂**明确不覆盖**。
///
/// ## 为什么不用正则
///
/// 本文件是**零依赖**的判据（只用 `std` ＋ `yeban_mcp::undo_session`）：
/// 引一个正则库会把依赖面扩大到一个连测试都不需要的第三方 crate。
fn bindings_with_separator(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = ["let ", "const ", "static "]
            .into_iter()
            .find_map(|keyword| trimmed.strip_prefix(keyword))
        else {
            continue;
        };
        let rest = rest.strip_prefix("mut ").unwrap_or(rest);
        let name: String = rest
            .chars()
            .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
            .collect();
        if name.is_empty() {
            continue;
        }
        let after_name = &rest[name.len()..];
        let Some(equals) = after_name.find('=') else {
            continue;
        };
        let value = &after_name[equals + 1..];
        // `==` / `=>` 之类不是赋值。
        if value.starts_with('=') || value.starts_with('>') {
            continue;
        }
        let Some(open) = value.find('"') else {
            continue;
        };
        let literal_start = open + 1;
        let Some(length) = value[literal_start..].find('"') else {
            continue;
        };
        let literal = &value[literal_start..literal_start + length];
        if literal.contains('/') || literal.contains('\\') {
            out.push((name, literal.to_owned()));
        }
    }
    out
}

/// 新臂：**参数是标识符**的路径比较（R93 洞①：`.ends_with(变量)`）。
///
/// [`unnormalized_path_comparison`] 只认"方法名后**紧跟**的字面量"（`.ends_with("…"`），
/// 因此 `let target = "domain/midi_input.rs";` ＋ `path.ends_with(target)` 这类**恒漏检**
/// —— 2026-10-11 那次真实缺陷（Windows 腿红）正是从这个洞过去的。
///
/// 本臂开火需要**同时**满足：
///
/// 1. 同行出现 `.ends_with(` 或 `.starts_with(` —— **仅这两个方法**
///    （⛔ 不含 `.contains`：它最常见的形态是集合成员判定，见已知绿样本 ①②④）；
/// 2. 参数是**裸标识符**（可带一个 `&`）且**紧跟 `)`**；
/// 3. 接收者沿用现有 [`PATHISH`] 口径（⛔ 不扩表）；
/// 4. 同行**没有**现有那四种规范化证据 —— 口径与 [`unnormalized_path_comparison`] 一致，
///    此处**刻意重复一遍**而不是抽公共函数：抽函数就要改动既有那一臂，
///    而本票的边界是"**只加不改**"；
/// 5. 该标识符 ∈ [`bindings_with_separator`]（**同文件**、且绑定的字面量**含分隔符**）。
///
/// 返回 `Some(诊断)`（含接收者、方法、标识符**及其绑定字面量**），否则 `None`。
fn unnormalized_variable_path_comparison(
    line: &str,
    bindings: &[(String, String)],
) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }
    for method in [".ends_with(", ".starts_with("] {
        let mut from = 0usize;
        while let Some(offset) = line[from..].find(method) {
            let at = from + offset;
            let argument = &line[at + method.len()..];
            let argument = argument.strip_prefix('&').unwrap_or(argument);
            let name: String = argument
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            if !name.is_empty() && argument[name.len()..].starts_with(')') {
                let receiver: String = line[..at]
                    .chars()
                    .rev()
                    .take_while(|character| {
                        character.is_ascii_alphanumeric() || *character == '_' || *character == '.'
                    })
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect();
                let pathish = PATHISH
                    .iter()
                    .any(|needle| receiver.to_ascii_lowercase().contains(needle));
                let normalized = line.contains("path_ends_with(")
                    || line.contains("normalized_path(")
                    || line.contains("replace('\\\\'")
                    || line.contains("replace(\"\\\\\"")
                    || (line.contains('/') && line.contains('\\'));
                if pathish
                    && !normalized
                    && let Some((_, literal)) = bindings.iter().find(|(known, _)| *known == name)
                {
                    let called = method.trim_start_matches('.').trim_end_matches('(');
                    return Some(format!(
                        "{receiver}.{called}({name}) —— 其中 {name} = \"{literal}\""
                    ));
                }
            }
            from = at + method.len();
        }
    }
    None
}

#[test]
fn dynamic_scans_declare_a_lower_bound() {
    // ⭐ R56：先喂**一条已知红**（用 read_dir 扫完直接断言，没有任何下界）与
    // **一条已知绿**（同一个扫描带 `scanned >= 30`）。
    let red = "fn bad() {\n    let entries = std::fs::read_dir(\".\").unwrap();\n    \
               for entry in entries.flatten() {\n        assert!(entry.path().exists());\n    }\n}\n";
    let helper_only =
        "fn all_sources() -> Vec<String> {\n    read_rust_sources(&[\"src\".into()])\n}\n";
    let green = "fn good() {\n    let files = read_rust_sources(&[\"src\".into()]);\n    \
                 let mut scanned = 0usize;\n    for (_path, text) in &files {\n        \
                 scanned += 1;\n        assert!(!text.is_empty());\n    }\n    \
                 assert!(scanned >= 30, \"扫描面太小\");\n}\n";
    let red_functions = functions(red);
    let green_functions = functions(green);
    assert_eq!(red_functions.len(), 1);
    assert!(
        red_functions[0].body.contains(DYNAMIC_SOURCES[1])
            && !declares_lower_bound(&red_functions[0].body),
        "已知红样本必须被判定为'没有下界'"
    );
    assert!(
        green_functions[0].body.contains("read_rust_sources")
            && declares_lower_bound(&green_functions[0].body),
        "已知绿样本必须被判定为'有下界'"
    );
    // 取数助手（只读集合、不做断言）不属于本判据的管辖范围。
    assert!(functions(helper_only)[0].body.contains("read_rust_sources"));
    assert!(!functions(helper_only)[0].body.contains("assert"));

    // 真源码：凡用动态集合源的函数都必须声明下界。
    let mut offenders: Vec<String> = Vec::new();
    let mut checked = 0usize;
    for (path, text) in all_sources() {
        for function in functions(&text) {
            if !all_dynamic_sources().any(|needle| function.body.contains(needle)) {
                continue;
            }
            // 只审"**同时断言了性质**"的函数：纯取数助手不承担下界责任。
            if !function.body.contains("assert") {
                continue;
            }
            checked += 1;
            if !declares_lower_bound(&function.body) {
                offenders.push(format!("{path}:{} {}", function.line, function.name));
            }
        }
    }
    // ⭐ R93 作用在**本判据自己**身上：被检查的函数数必须有下界（否则守卫恒绿）。
    assert!(
        checked >= 20,
        "本判据自己也要非真空：只检查到 {checked} 个用动态集合源的函数"
    );
    assert!(
        offenders.is_empty(),
        "凡遍历动态集合后断言性质的函数，必须同时断言被扫集合非空且 ≥ 下界（R93）：{offenders:#?}"
    );
}

#[test]
fn path_comparisons_are_normalized() {
    // ⭐ R56：已知红（`path` 接收者 + 含 `/` 的字面量，且没有任何规范化）／已知绿（三种合法写法）。
    let red = "assert!(sources_path.ends_with(\"tests/render_master.rs\"));";
    let greens = [
        "assert!(path_ends_with(&path, \"tests/render_master.rs\"));",
        "assert!(normalized_path(&path).ends_with(\"tests/render_master.rs\"));",
        "assert!(dir.ends_with(\"target/schema-samples\") || dir.ends_with(\"target\\\\schema-samples\"));",
        "assert!(path.replace('\\\\', \"/\").ends_with(\"tests/render_master.rs\"));",
    ];
    assert!(
        unnormalized_path_comparison(red).is_some(),
        "已知红样本必须被抓到"
    );
    for green in greens {
        assert!(
            unnormalized_path_comparison(green).is_none(),
            "已知绿样本不得被抓到：{green}"
        );
    }

    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    for (path, text) in all_sources() {
        for (index, line) in text.lines().enumerate() {
            scanned += 1;
            if let Some(receiver) = unnormalized_path_comparison(line) {
                offenders.push(format!("{path}:{} `{receiver}`", index + 1));
            }
        }
    }
    assert!(
        scanned >= 5_000,
        "扫描面太小（{scanned} 行）—— 路径过滤/文件枚举可能错了"
    );
    assert!(
        offenders.is_empty(),
        "路径比较必须先规范化（`path_ends_with` / `normalized_path` / 显式替换 `\\\\`）：{offenders:#?}"
    );
}

/// R93 洞①（新增臂）：**参数是变量**的路径比较也必须先规范化。
///
/// ## 这个洞是怎么被发现的
///
/// 2026-10-11：`the_payload_is_built_in_exactly_one_place` 在 **Windows** 腿上 panic
/// （`path.ends_with("domain/midi_input.rs")` 在 `\` 分隔符下恒 false）。
/// 修的时候顺手发现 [`unnormalized_path_comparison`] **抓不到**这一形态：
/// 它要求方法名后**紧跟**字面量，而那一行写的是 `.ends_with(target)`（参数是变量）
/// ⇒ 那处缺陷在本机（macOS/Linux）从头到尾**没有一个守卫看得见**。
///
/// ## 为什么这条判据**不能**靠"今天全仓 0 命中"来证明自己有效
///
/// 本臂在 `origin/main` 上的命中数**实测就是 0**（全仓普查，单位 = 调用点）。
/// 0 命中**既可能**是"没人犯这个错"，**也可能**是"这一臂根本没接线"——
/// 两者读数完全一样。所以下面先喂一条**内置已知红**（形状与那次真实缺陷**逐字同形**）
/// 并要求它**必须被抓到**；⛔ 只有那条断言过了，"0 命中"才是有意义的读数。
///
/// ⚠ 这一课是今晚刚交的学费：修 `path_ends_with` 时我第一版"只钉住助手、没钉住调用点"，
/// 注入验证是**全绿**的 —— "加了一条判据" ⛔ 不等于"这条判据真的在守"。
#[test]
fn variable_path_comparisons_are_normalized() {
    // ⭐ 内置**已知红**：与 2026-10-11 那次真实缺陷逐字同形
    //    （`let target = "domain/midi_input.rs";` ＋ `path.ends_with(target)`）。
    //    ⚠ 刻意写成**一整行**字符串：若拆成多行物理行，本文件自己就会在
    //    "真源码扫描"那一段里被自己抓到（那反而说明守卫是活的，但会让本判据自爆）。
    let red = "let target = \"domain/midi_input.rs\";\nlet (_, text) = sources.iter().find(|(path, _)| path.ends_with(target)).expect(\"必须扫到\");\n";
    let red_bindings = bindings_with_separator(red);
    assert_eq!(
        red_bindings.len(),
        1,
        "预扫必须抓到 `target` 的绑定（否则下面的'抓到'是假的）：{red_bindings:?}"
    );
    assert_eq!(red_bindings[0].0, "target");
    assert_eq!(red_bindings[0].1, "domain/midi_input.rs");
    let hit = red
        .lines()
        .find_map(|line| unnormalized_variable_path_comparison(line, &red_bindings))
        .expect("已知红样本必须被新臂抓到 —— 抓不到就说明这一臂没接线（空转）");
    assert!(
        hit.contains("path.ends_with(target)"),
        "诊断必须点名调用点：{hit}"
    );
    assert!(
        hit.contains("domain/midi_input.rs"),
        "诊断必须带出**绑定的字面量**，否则读的人无法判断它是不是路径：{hit}"
    );

    // ⭐ 已知绿：**(i)/(ii) 档那四种误报形态**，比较行**逐字取自全仓真实站点**
    //    （`origin/main`；见 `docs/ledger/` 的 2026-10-11 普查报告）。
    //    它们今天都必须**不被抓到**；将来若有人放宽 `PATHISH` 或把 `.contains` 也收进本臂，
    //    这几条会**先红**。
    let greens: [(&str, &str); 4] = [
        (
            "① const 数组的集合成员判定（extension_audit.rs:484）",
            "const CRATE_ROOT_FILES: [&str; 3] = [\"lib.rs\", \"main.rs\", \"mod.rs\"];\nif CRATE_ROOT_FILES.contains(&file_name) || !file_name.ends_with(\".rs\") {\n",
        ),
        (
            "② Vec<EntityId> 的集合成员判定（render.rs:1293）",
            "let source_nodes: Vec<EntityId> = Vec::new();\nif source_nodes.contains(&node) {\n",
        ),
        (
            "③ 单分量常量（samples.rs:987）",
            "const SAMPLES_DIR_NAME: &str = \"schema-samples\";\nlet dir = default_out_dir();\nassert!(dir.ends_with(SAMPLES_DIR_NAME));\n",
        ),
        (
            "④ 在源码文本里找一段摘要（fault_vocabulary.rs:1191）",
            "let own_source = include_str!(\"fault_vocabulary.rs\");\nassert!(own_source.contains(FROZEN_REASON_SURFACE_SHA256));\n",
        ),
    ];
    for (label, snippet) in greens {
        let bindings = bindings_with_separator(snippet);
        for line in snippet.lines() {
            assert!(
                unnormalized_variable_path_comparison(line, &bindings).is_none(),
                "已知绿样本不得被**新臂**抓到（{label}）：{line}"
            );
            assert!(
                unnormalized_path_comparison(line).is_none(),
                "已知绿样本也不得被**既有臂**抓到（{label}）：{line}"
            );
        }
    }

    // 真源码：新臂在本 crate（`src` ＋ `tests`）的命中数 —— **这一项就是交付读数**。
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    let mut bound = 0usize;
    for (path, text) in all_sources() {
        let bindings = bindings_with_separator(&text);
        bound += bindings.len();
        for (index, line) in text.lines().enumerate() {
            scanned += 1;
            if let Some(hit) = unnormalized_variable_path_comparison(line, &bindings) {
                offenders.push(format!("{path}:{} {hit}", index + 1));
            }
        }
    }
    // R93：被扫集合必须有下界（否则"0 命中"可能只是"什么都没扫"）。
    assert!(
        scanned >= 5_000,
        "扫描面太小（{scanned} 行）—— 路径过滤/文件枚举可能错了"
    );
    // ⭐ **预扫自己也要非真空**：一个含分隔符的绑定都扫不到 ⇒ 新臂永远不会开火。
    assert!(
        bound >= 5,
        "含分隔符的标识符绑定太少（{bound} 处）—— 预扫可能失效，新臂会退化成空转"
    );
    assert!(
        offenders.is_empty(),
        "参数是**变量**的路径比较也必须先规范化（洞①，2026-10-11 Windows 缺陷的形状）：{offenders:#?}"
    );
}

#[test]
fn the_guard_registry_is_not_empty_and_each_needle_exists() {
    // 守卫**自己**也要非真空：注册表不得为空，且每个针都必须真的出现在源码里
    // （否则"找不到 ⇒ 不检查"会让守卫悄悄退化）。
    //
    // ⚠ **第八批实测的假绿（R119/R120）**：第一版把**本文件**也算进"源码"，
    // 而针的字面量**就写在本文件的注册表里** ⇒ `joined.contains(needle)` **恒真**
    // （把 `"glob("` 改成 `"glob_probe("` 的注入**全绿**）。⇒ 现在**排除本文件**，
    // 并且要求"**别处**真的有命中"。
    assert!(!DYNAMIC_SOURCES.is_empty() && !NON_VACUITY_FORMS.is_empty() && !PATHISH.is_empty());
    let all = all_sources();
    // ⭐ R213：这个绝对下界是**裁定指定的例外**（R188/R199 通常禁"数量地板"）——
    // 它守的失败模式是"**被搜集合塌缩**"（太小），不是"缺陷抬高计数"。
    // 因此它必须①保留、②**根绑定到被搜集合自己**、③配一条行为臂
    // （把集合缩到界之下 ⇒ 必须变红，见注入 `A9-existbound-shrunk`）。
    let total = all.len();
    let mut scanned = 0usize;
    let joined = all
        .into_iter()
        .filter(|(path, _text)| !path.ends_with("non_vacuity_guard.rs"))
        .map(|(_path, text)| {
            scanned += 1;
            text
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        scanned >= 50,
        "排除本文件后的扫描面太小（{scanned} 个 .rs）—— 针的存在性判定会退化成假绿"
    );
    // ② **根绑定到被搜集合自己**：排除那一个定义文件后，必须**恰好**剩 `total - 1` 个。
    //    （被搜集合若塌缩，这里与上面的绝对界会**同时**收紧；上面的界负责"太小"这一失败模式。）
    assert_eq!(
        scanned,
        total.saturating_sub(1),
        "排除定义文件后必须恰好剩 `total - 1` 个 .rs（根绑定到被搜集合自己）：total={total} scanned={scanned}"
    );
    let mut miss: Vec<String> = Vec::new();
    for needle in DYNAMIC_SOURCES {
        if !joined.contains(needle) {
            miss.push(format!("动态集合源 `{needle}`"));
        }
    }
    let lower = joined.to_ascii_lowercase();
    for needle in PATHISH {
        if !lower.contains(needle) {
            miss.push(format!("路径前缀 `{needle}`"));
        }
    }
    for form in NON_VACUITY_FORMS {
        if !joined.contains(form) {
            miss.push(format!("非真空形态 `{form}`"));
        }
    }
    assert!(
        miss.is_empty(),
        "这些针在**别处**一个都找不到（守卫的针已经腐烂，或注册表写错了）：{miss:?}"
    );
    // ⭐ R188：**诊断**（⛔ 不是判据）—— 防御性动态源在本 crate 的命中数。
    let defensive_hits: Vec<(&str, usize)> = DEFENSIVE_SOURCES
        .iter()
        .map(|needle| (*needle, joined.matches(needle).count()))
        .collect();
    eprintln!("诊断（不参与判定）：防御性动态源命中数 = {defensive_hits:?}");
}
