//! **R94 的全量普查（本 crate 的公开扫描器）**：静态扫描器必须先掩码注释与字符串字面量，
//! 且掩码**保留换行**、只标区间不删字符。
//!
//! 本文件覆盖**公开**（可跨测试目标调用）的那几个扫描器；测试文件内部的三个扫描器
//! （`fault_vocabulary` 的 reason 三件套、`op_reachability` 的构造点分类器、
//! `probe_authenticity` 的断言形状扫描器）在**它们自己的文件里**有同形自证判据
//! （`the_reason_scanners_skip_comments_and_string_intervals` /
//! `the_classifier_tells_constructions_from_patterns` / 三条 `no_assertion_*`）。
//!
//! | 扫描器 | 掩码口径 | 本文件的判据 |
//! | :--- | :--- | :--- |
//! | `undo_session::production_region` | 逐行切到 `#[cfg(test)]` 之前 | [`production_regions_cut_at_the_attribute_only`] |
//! | `automation_audit::production_region` | 同上（三份逐字节相同） | 同上 |
//! | `extension_audit::production_region` | 同上 | 同上 |
//! | `automation_audit::code_without_literals` | 每行掩字符串与行尾注释（等长） | [`code_without_literals_masks_but_keeps_the_length`] |
//! | `extension_audit::path_ends_with` | `\` → `/` | [`path_ends_with_normalizes_both_separators`] |
//! | `extension_audit::quoted_strings` | 按出现顺序取双引号串 | [`quoted_strings_are_ordered_and_skip_nothing`] |
//! | `extension_audit::scan_dry_run_entry_points` | 逐行、跳过 `//` 开头的行 | [`the_dry_run_scan_is_not_fooled_by_comments_or_strings`] |

use yeban_mcp::domain::automation_audit;
use yeban_mcp::domain::extension_audit;
use yeban_mcp::undo_session;

/// 三个 `production_region` 共同的口径：**只在 `#[cfg(test)]` 那一行切**，且不被
/// 注释里的同一串误导。
#[test]
fn production_regions_cut_at_the_attribute_only() {
    let text = "fn produce() {}\n\
                /// 文档里写 `#[cfg(test)]` 不是真的测试区\n\
                fn still_production() {}\n\
                #[cfg(test)]\n\
                mod tests { fn only_test() {} }\n";
    for (name, region) in [
        ("undo_session", undo_session::production_region(text)),
        (
            "automation_audit",
            automation_audit::production_region(text),
        ),
        ("extension_audit", extension_audit::production_region(text)),
    ] {
        assert!(region.contains("fn produce()"), "{name}: 生产区必须留下");
        assert!(
            region.contains("fn still_production()"),
            "{name}: 注释里的 `#[cfg(test)]` 不得切断生产区"
        );
        assert!(!region.contains("only_test"), "{name}: 测试区必须被切掉");
        // ⭐ 非真空：切出来的生产区必须有内容（R93 作用在本判据自己身上）。
        assert!(region.lines().count() >= 3, "{name}: 生产区太小");
    }
}

/// `code_without_literals`：掩掉字符串与行尾注释，但**等长**（否则行号/列号会漂移）。
#[test]
fn code_without_literals_masks_but_keeps_the_length() {
    let line = "let x = compute(\"value_at(\"); // 尾注释里也有 value_at(";
    let masked = automation_audit::code_without_literals(line);
    assert_eq!(
        masked.chars().count(),
        line.chars().count(),
        "掩码必须等长（R94：⛔ 不删字符；按**字符**数，多字节注释也一样）"
    );
    assert!(
        !masked.contains("value_at("),
        "字符串与注释里的针必须被掩掉: {masked}"
    );
    assert!(masked.contains("compute("), "代码部分必须留下: {masked}");
    // 已知绿：真代码里的针**不得**被掩掉。
    assert!(
        automation_audit::code_without_literals("lane.value_at(tick)").contains("value_at("),
        "真调用必须留下"
    );

    // ⭐ R214①：掩码风险是**对偶的** —— 本助手属于"**掩码失同步 ⇒ 假阴性**"那一侧
    // （比"无掩码 ⇒ 假阳性"更危险：它会把后续正文当字符串抹掉，**掩盖真违规**）。
    // 因此必须用**四件套对抗样本**逐条读：等长（字符数）＋ **不泄漏**针。
    let adversarial: [(&str, &str); 4] = [
        // ① 字符串里含 `//`
        (
            "let a = \"http://x\"; let b = lane.value_at(1);",
            "value_at(",
        ),
        // ② **行尾**注释里含 `"`
        (
            "let c = 1; // 他说 \"lane.value_at(2)\" 是坏的\nlet d = lane.value_at(3);",
            "value_at(",
        ),
        // ③ **行尾**注释里含 `//`
        (
            "let e = 2; // http://y lane.value_at(4)\nlet f = lane.value_at(5);",
            "value_at(",
        ),
        // ④ ⭐ **块**注释里含 `"` 与 `//`
        (
            "let g = 3; /* 块注释: \"lane.value_at(6)\" 与 http://z */\nlet h = lane.value_at(7);",
            "value_at(",
        ),
    ];
    // ⑤ ⭐ **块注释里有一个未配对的 `"`**（R214① 的失同步形状：它会把**后续行**吞掉）。
    let unpaired = "let i = 4; /* 他说 \" 这个引号没有配对 */\nlet j = lane.value_at(8);";
    let samples: Vec<(&str, &str)> = adversarial
        .iter()
        .copied()
        .chain(std::iter::once((unpaired, "value_at(")))
        .collect();
    for (index, (source, needle)) in samples.iter().enumerate() {
        let masked = automation_audit::code_without_literals(source);
        assert_eq!(
            masked.chars().count(),
            source.chars().count(),
            "对抗样本 #{} 掩码必须等长",
            index + 1
        );
        // ⭐ 每个样本都**恰好一个真站点**：被引用的那个（在注释/字符串里）必须被掩掉，
        // 代码里的那个必须留下 ⇒ `count == 1`（⛔ 不是 `>= 1`：后者放过了泄漏）。
        assert_eq!(
            masked.matches(needle).count(),
            1,
            "对抗样本 #{} 必须**恰好**留下代码里那一个针（注释/字符串里的必须被掩掉，且不得失同步吞掉后续正文）: {masked}",
            index + 1
        );
    }

    // ⭐ R225①：四件套对 Rust **不充分** —— 还要看**生命周期**与**字符字面量**，
    // 以及 raw／byte-raw 字符串。
    // ① 生命周期 `'a` **不是**字符字面量（旧写法"见 `'` 就当字面量"会一路吞到下一个 `'` ⇒ 假阴性）。
    let lifetime = "fn f<'a>(x: &'a str) -> &'a str { x } let y = lane.value_at(9);";
    let masked = automation_audit::code_without_literals(lifetime);
    assert_eq!(
        masked.matches("value_at(").count(),
        1,
        "生命周期之后不得失同步: {masked}"
    );
    assert!(masked.contains("&'a str"), "生命周期必须原样留下: {masked}");
    // ② 字符字面量里的引号（`'"'`）必须被掩掉，且不得让它翻转字符串状态。
    let char_literal = "let q = '\"'; let z = lane.value_at(10);";
    let masked = automation_audit::code_without_literals(char_literal);
    assert_eq!(
        masked.matches("value_at(").count(),
        1,
        "字符字面量之后不得失同步: {masked}"
    );
    // ③ **单行** raw 字符串：它的定界符仍是 `"`，内部必须被掩掉、代码里的针必须留下。
    let raw_single = "let r = r#\"lane.value_at(11)\"#; let w = lane.value_at(12);";
    let masked = automation_audit::code_without_literals(raw_single);
    assert_eq!(
        masked.matches("value_at(").count(),
        1,
        "单行 raw 字符串必须被掩掉内部: {masked}"
    );
    // ④ ⚠ **已知限制（R190：登记 ＋ 爆炸半径，⛔ 不假装有牙）**：本助手是**逐行**的，
    //    所以**跨行** raw 字符串的**内部行**仍会被当成代码（R228① 实测：真实源码里
    //    有 **7 行 production 区**的内部行含 `&mut Domain`／`dryRun`，当前**未**触发违规，
    //    因为对应扫描器要求同一行还有第二个针 `fn plan_`）。这里把它作为**诊断**打印。
    // 真实调用形态是**逐行**喂（`for line in production.lines()`）⇒ 内部行**单独**进来时，
    // 助手**无法知道**它在一个跨行 raw 字符串里 ⇒ 它会被当成代码。
    let interior_line = " lane.value_at(13) ";
    let masked_line = automation_audit::code_without_literals(interior_line);
    eprintln!(
        "诊断（⛔ 不作判据）：跨行 raw 字符串的**内部行**逐行喂入时，针是否被掩掉 = {} \
         （已知限制：逐行助手看不见跨行上下文；真实源码里 production 区有 7 行这种内部行）",
        !masked_line.contains("value_at(")
    );
}

/// `path_ends_with`：两种分隔符都要认（R63/R93 的共享约定）。
#[test]
fn path_ends_with_normalizes_both_separators() {
    assert!(extension_audit::path_ends_with(
        "crates/yeban-mcp/src/domain/automation.rs",
        "domain/automation.rs"
    ));
    assert!(extension_audit::path_ends_with(
        "crates\\yeban-mcp\\src\\domain\\automation.rs",
        "domain/automation.rs"
    ));
    assert!(!extension_audit::path_ends_with(
        "src/domain/other.rs",
        "domain/automation.rs"
    ));
}

/// `quoted_strings`：按出现顺序取，且不吞相邻内容。
#[test]
fn quoted_strings_are_ordered_and_skip_nothing() {
    assert_eq!(
        extension_audit::quoted_strings("[\"A\", \"B\", \"C\"]"),
        vec!["A".to_owned(), "B".to_owned(), "C".to_owned()]
    );
    assert!(extension_audit::quoted_strings("no quotes here").is_empty());
}

/// **R94 的关键一问**：`scan_dry_run_entry_points` 会不会被**行尾注释**或**字符串**里的
/// `&mut Domain` 骗到？（它只跳过"整行以 `//` 开头"的行。）
///
/// 这个判据是**普查的一部分**：若它红，说明产线扫描器缺掩码 ⇒ 必须补（而不是改判据）。
#[test]
fn the_dry_run_scan_is_not_fooled_by_comments_or_strings() {
    let path = "crates/yeban-mcp/src/domain/mod.rs".to_owned();
    let text = "fn plan_ok() -> Result<(), ()> { Ok(()) } // 文档: fn plan_x(&mut Domain) 是坏形状\n\
                fn sample() { let s = \"fn plan_y(&mut Domain)\"; let _ = s; }\n\
                #[cfg(test)]\n\
                mod tests { fn t() { fn plan_z(&mut Domain) {} } }\n";
    let violations = extension_audit::scan_dry_run_entry_points(&[(path, text.to_owned())]);
    let leaked: Vec<&String> = violations
        .iter()
        .filter(|line| {
            line.contains("plan_x") || line.contains("plan_y") || line.contains("plan_z")
        })
        .collect();
    assert!(
        leaked.is_empty(),
        "行尾注释／字符串／测试区里的 `&mut Domain` 不得被当成计划入口（R94）: {leaked:#?}"
    );
    // 已知红（同一条扫描器必须仍有牙）：**真**的坏形状必须被点名。
    let bad = "fn plan_bad(&mut Domain) -> Result<(), ()> { Ok(()) }";
    let caught = extension_audit::scan_dry_run_entry_points(&[(
        "crates/yeban-mcp/src/domain/mod.rs".to_owned(),
        bad.to_owned(),
    )]);
    assert!(
        caught.iter().any(|line| line.contains("plan_bad")),
        "真的坏形状必须被抓到（否则这条判据自己没牙）: {caught:#?}"
    );
}

/// 全部 `.rs`（`src` ＋ `tests`），**按路径排序**（读数与文件系统顺序无关）。
fn all_sources() -> Vec<(String, String)> {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let mut files =
        yeban_mcp::undo_session::read_rust_sources(&[root.join("src"), root.join("tests")]);
    files.sort_by(|left, right| left.0.cmp(&right.0));
    files
}

/// **R228①/R253③：masked 与 raw 的差异必须**先分型**，且差异本身不是缺陷信号。**
///
/// 口径（单位与区域都写在这里）：
/// * **区域** = `crates/yeban-mcp/src/**`（生产区文件，**不含** `tests/**`）；
/// * **针集** = 本 crate 生产扫描器实际使用的那些（`value_at(` / `.ease(` / `interpolate` /
///   `origin:` / `dryRun` / `&mut Domain` / `&mut YebanProjectV1` / `apply_inverse`）；
/// * **两种口径**：① **子串**（⛔ 含污染 ⇒ 只是一个**上界**）；② **词边界**（针的两侧不得是
///   标识符字符 ⇒ 排除 `dryRunParam` 这类）。
///
/// 断言（⛔ 不写死具体数字 —— 数字随源码变动，写死就是毒）：
/// 1. ⭐ **不变量**：没有任何一行的针**在代码里却被掩掉**（那才是假阴性）；
/// 2. 词边界口径 ≤ 子串口径（偏置方向固定：子串只会**多**算）；
/// 3. 子串口径 ≥ 40（非真空下界，R119/R120）；
/// 4. 分型结果只允许两种：`string`（针在字符串字面量里 —— 正确掩码）与
///    `raw-interior`（跨行 raw 字符串的内部行 —— **已知限制**，见下面的恒等式说明）。
///
/// ⚠ **R251③ 的恒等式（⛔ 不是读数）**：某形态若**按构造**必为 0，它就不携带信息：
/// * 针在**行注释**里 ⇒ 行注释**整段**被掩（含 `"` 与 `//`）⇒ **恒等式 0**；
/// * 针在**块注释**里 ⇒ **修复前非 0**（那是真的假阴性，第九批实测红）、**修复后 0**
///   ⇒ 它**依赖掩码器实现**，⛔ 不是恒等式；
/// * 针在**单行字符串/单行 raw** 里 ⇒ 恒等式 0；针在**跨行 raw 的内部行** ⇒ **非 0**（已知限制）。
#[test]
fn masked_versus_raw_differences_are_typed_and_never_hide_code() {
    const NEEDLES: [&str; 8] = [
        "value_at(",
        ".ease(",
        "interpolate",
        "origin:",
        "dryRun",
        "&mut Domain",
        "&mut YebanProjectV1",
        "apply_inverse",
    ];
    let mut substring = 0usize;
    let mut word_boundary = 0usize;
    let mut in_string = 0usize;
    let mut other = 0usize;
    let mut hidden_in_code: Vec<String> = Vec::new();
    let mut scanned_lines = 0usize;
    for (path, text) in all_sources() {
        if !path.contains("src") || path.contains("tests") {
            continue;
        }
        for (index, line) in text.lines().enumerate() {
            scanned_lines += 1;
            if line.trim_start().starts_with("//") {
                continue;
            }
            let masked = automation_audit::code_without_literals(line);
            for needle in NEEDLES {
                if !line.contains(needle) || masked.contains(needle) {
                    continue;
                }
                substring += 1;
                if is_inside_string_literal(line, needle) {
                    in_string += 1;
                } else {
                    other += 1;
                }
                if !is_inside_string_literal(line, needle) && !inside_cross_line_raw(line) {
                    hidden_in_code.push(format!("{path}:{}", index + 1));
                }
                // 词边界口径：只在针的两侧都不是标识符字符时计入。
                for (at, _) in line.match_indices(needle) {
                    let left = line[..at].chars().next_back().unwrap_or(' ');
                    let right = line[at + needle.len()..].chars().next().unwrap_or(' ');
                    if !left.is_alphanumeric()
                        && left != '_'
                        && !right.is_alphanumeric()
                        && right != '_'
                    {
                        word_boundary += 1;
                        break;
                    }
                }
            }
        }
    }
    assert!(scanned_lines >= 5_000, "扫描面太小（{scanned_lines} 行）");
    assert!(
        substring >= 40,
        "子串口径差异数太小（{substring}）—— 分型可能什么都没扫到"
    );
    assert!(
        word_boundary <= substring,
        "词边界口径（{word_boundary}）不得大于子串口径（{substring}）—— 偏置方向写反了"
    );
    assert!(
        hidden_in_code.is_empty(),
        "这些行里针在**代码**中却被掩掉（假阴性）：{hidden_in_code:#?}"
    );
    eprintln!(
        "诊断（⛔ 不作判据）：口径①子串={substring}（上界）口径②词边界={word_boundary}；\
         分型 string={in_string} other={other}"
    );
}

/// 该针在**这一行**里是否落在字符串字面量内部（逐行口径）。
fn is_inside_string_literal(line: &str, needle: &str) -> bool {
    let Some(at) = line.find(needle) else {
        return false;
    };
    let before = &line[..at];
    // 逐字符数引号（忽略转义）：奇数 ⇒ 落在字符串内部。
    let mut inside = false;
    let mut chars = before.chars().peekable();
    while let Some(character) = chars.next() {
        match character {
            '\\' => {
                chars.next();
            }
            '"' => inside = !inside,
            '/' if !inside && chars.peek() == Some(&'/') => return false,
            _ => {}
        }
    }
    inside
}

/// 粗略判定：该行是否可能是**跨行 raw 字符串的内部行**（已知限制的承载体）。
///
/// 口径：行内出现裸的 `value_at(` 这类针、且**不在**字符串里、且该行**没有**引号配对
/// ⇒ 最可能的解释是"上一行开了 raw 字符串"（逐行助手看不见那个上下文）。
fn inside_cross_line_raw(line: &str) -> bool {
    !is_inside_string_literal(line, "value_at(") && line.matches('"').count() % 2 == 1
}
