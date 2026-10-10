//! **R69/R75 探针自审 ＋ R77 硬断言普查**（第五批）。
//!
//! ## 为什么需要这一份
//!
//! ### R69/R75：断言的两侧"在这条路径上真的会不同吗"
//!
//! * **假探针**（R69 形状①）：`assert_ne!(A::X, A::Y)` 里两个**不同变体** ——
//!   **判别式先判完**，被比较类型的 `PartialEq` 即使被削弱成"只比判别式"它照过 ⇒
//!   它**证明不了载荷有牙**（⚠ 它仍是真断言，可以保留，但**必须加注**）。
//! * **真探针**：**同一变体、不同载荷**（`T{1,1}` vs `T{1,2}`）✓；
//!   结构体**单字段差异**（无判别式）✓。
//! * **必然失败的断言**（R75）：两侧在**构造上相等** ⇒ 判据恒假 ⇒ 与被测代码无关地永远红。
//! * **同义反复**（R80）：两侧是**不同的字面量**（`assert_ne!(0usize, 2usize)`）⇒ 恒真 ⇒
//!   看起来像真断言却什么都没测到 ⇒ 正确形态是**读夹具表**（`assert_ne!(cases[0].1, cases[1].1)`）。
//!
//! ## 各种形状的**强度**（⛔ 不许把弱探针当成强证据）
//!
//! | 形状 | 判定 | 强度 |
//! | :--- | :--- | :--- |
//! | 两侧同文 | 恒真／恒假 | **0**（恒假 ⇒ 必然红；恒真 ⇒ 零信息） |
//! | 两侧是字面量 | 恒真 | **0**（R80） |
//! | 同一枚举两个无载荷变体 | 判别式先判完 | **弱**（只证明判别式，⛔ 不证明载荷） |
//! | `<常量> != <常量>` | 两个**命名**常量 | **弱**（只证明这两个常量当前指向不同的值；⛔ 与被比较类型的 `==` 无关） |
//! | 同一变体不同载荷 / 结构体单字段差异 | 真探针 | **强**（`==` 被削弱就会红） |
//! | 自反探针 `f(x) == f(x)` | 恒真 | **很弱**（只挡"函数不纯"；必须加注说明强度） |
//!
//! ### R77：凡"我们定字节"的硬断言，必须逐**被调函数**问是否含超越函数
//!
//! `sin`/`cos`/`exp`/`ln`/`powf`/`log10` 是宿主 libm ⇒ **跨平台不保证逐位相同**；
//! `sqrt`/乘除/比较属 IEEE 精确类 ⇒ 安全。⚠ 本地全绿不是证据（本机就是冻结架构）。
//!
//! 本 crate 的读数是**两层**的：
//! * **本 crate 自己的代码**：生产区超越函数调用点 = **0**（本文件第 3 条判据钉住）；
//! * **被本 crate 调用的上游**（`yeban-render` 的合成/抖动/母带链）：含超越函数 ⇒
//!   凡断言其产物字节的判据都是**跨平台假设**，必须登记（本文件第 4 条判据钉住清单）。

use std::collections::BTreeSet;
use std::path::PathBuf;

use yeban_mcp::undo_session::read_rust_sources;

/// 断言两测的三种"没有牙"的形状。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum ProbeShape {
    /// 两侧**文本相同** ⇒ 恒真／恒假（R75 那一族）。
    SameSides,
    /// `assert_ne!(<常量A>, <常量B>)` 而**没有第三个实参**（说明"它只证明两个字面量不同"）。
    ConstantPairWithoutMessage,
    /// 同上，但**带了说明**（已加注 ⇒ 合格形状）。
    ConstantPairWithMessage,
    /// **两侧都是字面量**（`assert_ne!(0usize, 2usize)`）⇒ 恒真 ⇒ 什么也没测到（R80）。
    BothSidesAreLiterals,
    /// `assert_ne!` 两侧是**同一枚举的两个不同无载荷变体** ⇒ 判别式先判完（R69 形状①）。
    EnumVariantInequality,
}

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 生产区（`#[cfg(test)]` 之前）。
fn production_region(text: &str) -> String {
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("#[cfg(test)]") {
            break;
        }
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// 全部 `.rs`（`src` ＋ `tests`），**按路径排序**（判据必须与文件系统顺序无关）。
fn all_sources() -> Vec<(String, String)> {
    let root = manifest_dir();
    let mut files = read_rust_sources(&[root.join("src"), root.join("tests")]);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// **路径规范化**（R63）：Windows 的 `SourceFile` 路径用 `\` 分隔 ⇒ 任何
/// `contains("/src/")`／`ends_with("tests/x.rs")` 在 Windows 上都会**假失败或真空通过**。
/// ⚠ 这是本文件**跨平台**才暴露的形态：Linux 腿永远看不到（windows 腿实测红过一次）。
fn normalized_path(path: &str) -> String {
    path.replace('\\', "/")
}

fn normalized(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// 从一个 `(` 出发做字符串感知的括号配对，返回配对的 `)` 的下标。
fn matching_close(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    let mut index = open;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'"' => in_string = true,
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// 顶层逗号切分（字符串／括号／方括号／花括号都算一层）。
fn top_level_args(inner: &str) -> Vec<String> {
    let bytes = inner.as_bytes();
    let mut args = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;
    let mut in_string = false;
    let mut escaped = false;
    for &byte in bytes {
        if in_string {
            current.push(char::from(byte));
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            continue;
        }
        match byte {
            b'"' => {
                in_string = true;
                current.push('"');
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                current.push(char::from(byte));
            }
            b')' | b']' | b'}' => {
                depth -= 1;
                current.push(char::from(byte));
            }
            b',' if depth == 0 => {
                args.push(current.trim().to_owned());
                current.clear();
            }
            _ => current.push(char::from(byte)),
        }
    }
    if !current.trim().is_empty() {
        args.push(current.trim().to_owned());
    }
    args
}

/// 是不是一个"全大写下划线"的常量路径（`SCENE_FIELD`、`domain::SEC_X`）。
fn is_screaming_const(expr: &str) -> bool {
    !expr.is_empty()
        && expr.split("::").all(|segment| {
            !segment.is_empty()
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
                && segment.bytes().any(|byte| byte.is_ascii_uppercase())
        })
}

/// 是不是一个**无载荷的变体路径**（`LaneKind::DeviceParam`、`Ok`、`err::Kind::X`）。
///
/// 返回 `Some(路径前缀)` 表示"是"；`None` 表示"不是"（带括号／字面量／变量都不算）。
fn bare_variant(expr: &str) -> Option<String> {
    let expr = expr.trim();
    if expr.is_empty() || expr.contains(['(', '"', ' ', '!', '.']) {
        return None;
    }
    let mut segments: Vec<&str> = expr.split("::").collect();
    let last = segments.pop()?;
    // 变体是 CamelCase：全大写下划线是**常量**，不是变体（`SCENE_FIELD` 不算判别式探针）。
    let screaming = last.contains('_') || last.bytes().all(|byte| !byte.is_ascii_lowercase());
    let ok_last = !screaming
        && last
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase());
    let ok_rest = segments.iter().all(|segment| {
        !segment.is_empty()
            && segment
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
    });
    (ok_last && ok_rest).then(|| segments.join("::"))
}

/// 把**注释**与**字符串字面量**的内容替换成空格（长度与换行保持不变）。
///
/// 为什么必须掩码：本判据自己就在**字符串字面量**里放了已知红样本
/// （`probe_shapes("assert_ne!(lanes, lanes);")`），文档里也引用了断言的写法；
/// 不掩码的话，扫描器会把**它自己举的例子**当成真源码里的违规 ⇒ 自指假阳性。
fn masked_code(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = vec![b' '; bytes.len()];
    let mut index = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
            // ⚠ **换行永远保留**：字符串的续行符（`\` 结尾）会把 `\n` 也掩掉，
            // 那样后面所有断言的**行号**都会整体漂移（本轮实测：真站点 10170 被报成 10093）。
            if byte == b'\n' {
                out[index] = byte;
            }
            index += 1;
            continue;
        }
        if byte == b'"' {
            in_string = true;
            index += 1;
            continue;
        }
        if byte == b'/' && bytes.get(index + 1) == Some(&b'/') {
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            continue;
        }
        out[index] = byte;
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// 是不是一个**字面量**（数字 / 字符串 / 字符 / 布尔 / 带后缀的数字）。
///
/// R80：两侧都是字面量的 `assert_ne!` **恒真**（"不同的字面量永远不相等"），
/// 它看起来像真断言却什么都没测到 —— 要改成**读夹具表**（`assert_ne!(cases[0].1, cases[1].1)`）。
fn is_literal(expr: &str) -> bool {
    let text = expr.trim();
    if text.is_empty() {
        return false;
    }
    if matches!(text, "true" | "false") {
        return true;
    }
    if (text.starts_with('"') && text.ends_with('"') && text.len() >= 2)
        || (text.starts_with('\'') && text.ends_with('\'') && text.len() >= 3)
    {
        return true;
    }
    let first = text.bytes().next().unwrap_or(b'x');
    if first.is_ascii_digit()
        || (first == b'-' && text.len() > 1 && text.as_bytes()[1].is_ascii_digit())
    {
        return text.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'.' || byte == b'-'
        });
    }
    false
}

/// 扫一个文件里所有 `assert_eq!`／`assert_ne!` 的"没有牙"形状。
fn probe_shapes(text: &str) -> Vec<(usize, ProbeShape)> {
    let masked = masked_code(text);
    let bytes = masked.as_bytes();
    let real = text.as_bytes();
    let mut out = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        let rest = &masked[index..];
        let found = ["assert_eq!(", "assert_ne!("]
            .iter()
            .filter_map(|needle| rest.find(needle).map(|at| (at, *needle)))
            .min_by_key(|(at, _needle)| *at);
        let Some((at, needle)) = found else {
            break;
        };
        let macro_at = index + at;
        let open = macro_at + needle.len() - 1;
        let Some(close) = matching_close(bytes, open) else {
            break;
        };
        let args = top_level_args(&text[open + 1..close.min(real.len())]);
        let line = masked[..macro_at].matches('\n').count() + 1;
        if args.len() >= 2 {
            let left = normalized(&args[0]);
            let right = normalized(&args[1]);
            if left == right {
                out.push((line, ProbeShape::SameSides));
            } else if needle == "assert_ne!(" {
                if is_literal(&left) && is_literal(&right) {
                    out.push((line, ProbeShape::BothSidesAreLiterals));
                } else if is_screaming_const(&left) && is_screaming_const(&right) {
                    out.push((
                        line,
                        if args.len() < 3 {
                            ProbeShape::ConstantPairWithoutMessage
                        } else {
                            ProbeShape::ConstantPairWithMessage
                        },
                    ));
                } else if let (Some(prefix_left), Some(prefix_right)) =
                    (bare_variant(&left), bare_variant(&right))
                    && prefix_left == prefix_right
                {
                    out.push((line, ProbeShape::EnumVariantInequality));
                }
            }
        }
        index = close + 1;
    }
    out
}

/// 超越函数调用点（`.sin(` / `.powf(` …）—— 宿主 libm ⇒ 跨平台不保证逐位相同。
const TRANSCENDENTAL: [&str; 15] = [
    "sin", "cos", "tan", "asin", "acos", "atan", "atan2", "sinh", "cosh", "tanh", "exp", "ln",
    "log2", "log10", "powf",
];

fn transcendental_sites(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for (index, line) in production_region(text).lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        for name in TRANSCENDENTAL {
            if line.contains(&format!(".{name}(")) {
                out.insert(format!("{}:{}", index + 1, name));
            }
        }
    }
    out
}

#[test]
fn no_assertion_compares_a_side_with_itself() {
    // R56：先喂已知红＋已知绿。
    let red = "assert_ne!(lanes, lanes);";
    let green = "assert_ne!(before, after);\nassert_eq!(count, 3);";
    assert_eq!(
        probe_shapes(red)
            .iter()
            .filter(|(_line, shape)| *shape == ProbeShape::SameSides)
            .count(),
        1,
        "已知红样本必须被抓到"
    );
    assert!(
        probe_shapes(green)
            .iter()
            .all(|(_line, shape)| *shape != ProbeShape::SameSides),
        "已知绿样本不得被抓到"
    );
    // 真源码分两类（R75 的是**恒假**那一类）：
    // * `assert_ne!(X, X)` ⇒ 恒假 ⇒ **必然失败** ⇒ 必须为零（R75 的笔误形状）；
    // * `assert_eq!(X, X)` ⇒ 恒真 ⇒ 它是"**纯函数自比**"（同输入两次必须同值 ——
    //   函数若不纯，它会红，所以它**有牙**，只是牙长在确定性上）⇒ 允许，但**必须加注**
    //   （与 R69 对假探针的处置同形：可以保留，必须在源码里标明它证明的是什么）。
    let mut always_false: Vec<String> = Vec::new();
    let mut unannotated: Vec<String> = Vec::new();
    let mut purity_probes = 0usize;
    for (path, text) in all_sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (line, shape) in probe_shapes(&text) {
            if shape != ProbeShape::SameSides {
                continue;
            }
            let source_line = lines
                .get(line.saturating_sub(1))
                .copied()
                .unwrap_or_default();
            let is_negated = source_line.contains("assert_ne!");
            // 加注可以在**上面连续若干行注释**里（注释可多行）⇒ 向上扫到第一个非注释行为止。
            let mut annotated = source_line.contains("R75:");
            let mut cursor = line.saturating_sub(2);
            while let Some(previous) = lines.get(cursor) {
                let trimmed = previous.trim_start();
                if trimmed.starts_with("//") {
                    if previous.contains("R75:") || previous.contains("纯函数自比") {
                        annotated = true;
                        break;
                    }
                    if cursor == 0 {
                        break;
                    }
                    cursor -= 1;
                    continue;
                }
                break;
            }
            if is_negated {
                always_false.push(format!("{path}:{line}"));
            } else if annotated {
                purity_probes += 1;
            } else {
                unannotated.push(format!("{path}:{line}"));
            }
        }
    }
    assert!(
        always_false.is_empty(),
        "`assert_ne!(X, X)` 恒假 ⇒ 与被测代码无关地永远红（R75 的笔误形状）：{always_false:?}"
    );
    assert!(
        unannotated.is_empty(),
        "`assert_eq!(X, X)` 是恒真的纯函数自比：它证明的是**确定性**，必须在上一行注明 \
         （`R75: 纯函数自比…`）：{unannotated:?}"
    );
    assert!(
        purity_probes > 0,
        "本 crate 应当有已加注的纯函数自比探针（否则这条判据是空的）"
    );
}

#[test]
fn enum_variant_inequality_probes_are_absent_or_annotated() {
    // R56：已知红（同一枚举两个无载荷变体）／已知绿（同一变体不同载荷、结构体单字段）。
    let red = "assert_ne!(LaneKind::TrackVolume, LaneKind::TrackPan);";
    let green = "assert_ne!(Truncated { kept: 1 }, Truncated { kept: 2 });\nassert_ne!(left.samples, right.samples);";
    assert_eq!(
        probe_shapes(red)
            .iter()
            .filter(|(_line, shape)| *shape == ProbeShape::EnumVariantInequality)
            .count(),
        1,
        "已知红样本必须被抓到"
    );
    assert!(
        probe_shapes(green)
            .iter()
            .all(|(_line, shape)| *shape != ProbeShape::EnumVariantInequality),
        "已知绿样本不得被抓到"
    );
    // 真源码：0 处（R69 形状①）。将来若真的引入，必须在同一行或上一行写 `R69:` 加注。
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in all_sources() {
        let lines: Vec<&str> = text.lines().collect();
        for (line, shape) in probe_shapes(&text) {
            if shape != ProbeShape::EnumVariantInequality {
                continue;
            }
            let annotated = lines
                .get(line.saturating_sub(2))
                .is_some_and(|previous| previous.contains("R69:"))
                || lines
                    .get(line.saturating_sub(1))
                    .is_some_and(|own| own.contains("R69:"));
            if !annotated {
                offenders.push(format!("{path}:{line}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "`assert_ne!` 落在同一枚举的两个无载荷变体上 ⇒ 判别式先判完（假探针）；\
         要么改成同一变体不同载荷的真探针，要么加 `R69:` 注明它只证明判别式：{offenders:?}"
    );
}

#[test]
fn no_assertion_compares_two_literals() {
    // R56：先喂已知红＋已知绿。
    let red =
        "assert_ne!(0usize, 2usize, \"两个夹具必须是不同的样本数\");\nassert_ne!(\"a\", \"b\");";
    let green = "assert_ne!(cases[0].1, cases[1].1);\nassert_ne!(code, \"NOT_IMPLEMENTED\");";
    assert_eq!(
        probe_shapes(red)
            .iter()
            .filter(|(_line, shape)| *shape == ProbeShape::BothSidesAreLiterals)
            .count(),
        2,
        "已知红样本必须被抓到两条"
    );
    assert!(
        probe_shapes(green)
            .iter()
            .all(|(_line, shape)| *shape != ProbeShape::BothSidesAreLiterals),
        "已知绿样本不得被抓到（读**夹具表**的探针必须留在绿）"
    );
    // 真源码：0 处（R80 的第三种空判据形状）。
    let mut offenders: Vec<String> = Vec::new();
    for (path, text) in all_sources() {
        for (line, shape) in probe_shapes(&text) {
            if shape == ProbeShape::BothSidesAreLiterals {
                offenders.push(format!("{path}:{line}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "两侧都是字面量的断言**恒真** ⇒ 什么都没测到（R80）。改成读**夹具表**或读被测值：{offenders:?}"
    );
}

#[test]
fn constant_pair_inequality_probes_carry_a_message() {
    // R56：已知红（常量对没有说明）／已知绿（常量对带说明）。
    assert_eq!(
        probe_shapes("assert_ne!(SCENE_FIELD, SECTION_FIELD);")
            .iter()
            .filter(|(_line, shape)| *shape == ProbeShape::ConstantPairWithoutMessage)
            .count(),
        1,
        "已知红样本必须被抓到"
    );
    assert_eq!(
        probe_shapes("assert_ne!(SCENE_FIELD, SECTION_FIELD, \"场景不是段落\");")
            .iter()
            .filter(|(_line, shape)| *shape == ProbeShape::ConstantPairWithMessage)
            .count(),
        1,
        "已知绿样本必须被识别成'带说明的常量对'"
    );
    let mut offenders: Vec<String> = Vec::new();
    let mut total = 0usize;
    for (path, text) in all_sources() {
        for (line, shape) in probe_shapes(&text) {
            match shape {
                ProbeShape::ConstantPairWithoutMessage => {
                    total += 1;
                    offenders.push(format!("{path}:{line}"));
                }
                ProbeShape::ConstantPairWithMessage => total += 1,
                _ => {}
            }
        }
    }
    assert!(
        total > 0,
        "本 crate 应当有「两个字面量不同」的探针（否则这条判据是空的）"
    );
    assert!(
        offenders.is_empty(),
        "`assert_ne!(<常量>, <常量>)` 只证明两个字面量不同，必须带说明：{offenders:?}"
    );
}

#[test]
fn the_crates_own_transcendental_call_sites_are_the_registered_list() {
    // R56：已知红（`.powf(`）／已知绿（`sqrt`、乘除、比较）。
    let red = "let gain = 10.0_f32.powf(db / 20.0);";
    let green = "let r = value.sqrt();\nlet half = value * 0.5;\nif a > b { }";
    assert_eq!(transcendental_sites(red).len(), 1, "已知红样本必须被抓到");
    assert!(
        transcendental_sites(green).is_empty(),
        "已知绿样本不得被抓到（`sqrt`/乘除/比较属 IEEE 精确类）"
    );
    // 真源码：生产区必须**一个都没有**。
    let mut found: BTreeSet<String> = BTreeSet::new();
    let mut scanned = 0usize;
    for (path, text) in all_sources() {
        if !normalized_path(&path).contains("/src/") {
            continue;
        }
        scanned += 1;
        for site in transcendental_sites(&text) {
            found.insert(format!("{path} {site}"));
        }
    }
    // ⭐ **非真空**断言：路径过滤写错（例如漏了规范化）会让扫描面变成空集 ⇒ 判据**恒绿**。
    assert!(
        scanned >= 30,
        "生产区扫描面太小（{scanned} 个文件）—— 路径过滤可能写错了（R63/R70③）"
    );
    assert!(
        found.is_empty(),
        "本 crate 生产区不得出现超越函数（它们会让'我们定字节'的硬断言变成跨平台假设）：{found:?}"
    );
}

/// **上游 DSP 硬断言登记表**（R77②：本地全绿 = 未验证的跨平台假设 ⇒ 必须登记）。
///
/// 这些判据断言的是 `yeban-render` 合成／抖动／母带链的**产物字节**或**样本摘要**，
/// 而上游链里有超越函数（振荡器相位、抖动）⇒ 它们**不是**本 crate 能自己证明的
/// 位级契约，只能靠 **CI 两个平台**（本 crate 的 windows 腿）当真。
/// 本判据的作用：**任何人改名／删掉这些判据都会红**，从而不许清单悄悄腐烂。
const UPSTREAM_DSP_HARD_ASSERTIONS: [(&str, &str); 5] = [
    (
        "two_renders_of_the_same_project_are_byte_identical",
        "两次自比 ＋ 冻结的 DEFAULT_MASTER_DIGEST / DEFAULT_MASTER_SHA256（跨越上游合成/抖动链）",
    ),
    (
        "the_default_rendering_path_stays_bit_identical_with_a_loudness_target_attached",
        "响度目标挂上后默认路径的位级不变（跨越上游母带链）",
    ),
    (
        "the_requested_format_changes_the_container_but_not_the_audio",
        "换格式只改容器、不改音频（跨越上游编码链）",
    ),
    (
        "only_a_loop_that_covers_the_whole_placement_is_silent",
        "整段覆盖的循环才静音（冻结样本摘要）",
    ),
    (
        "ratchet_renders_are_bit_identical_and_the_default_path_is_unchanged",
        "连击渲染的位级一致 ＋ 默认路径不变（冻结样本摘要）",
    ),
];

#[test]
fn upstream_dsp_hard_assertions_are_registered() {
    let files = all_sources();
    let render_tests = files
        .iter()
        .find(|(path, _text)| normalized_path(path).ends_with("tests/render_master.rs"))
        .map(|(_path, text)| text.clone())
        .expect("tests/render_master.rs 必须在源码集合里（路径按 R63 规范化后比较）");
    for (name, why) in UPSTREAM_DSP_HARD_ASSERTIONS {
        assert!(
            render_tests.contains(&format!("fn {name}(")),
            "登记表里的 `{name}` 在 tests/render_master.rs 里找不到了（{why}）"
        );
    }
    // 反向：登记表不得为空，且每条都有说明（表与代码不许各说各话）。
    assert_eq!(UPSTREAM_DSP_HARD_ASSERTIONS.len(), 5);
    assert!(
        UPSTREAM_DSP_HARD_ASSERTIONS
            .iter()
            .all(|(_name, why)| !why.is_empty())
    );
}
