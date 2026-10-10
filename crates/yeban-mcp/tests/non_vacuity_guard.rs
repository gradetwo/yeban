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
