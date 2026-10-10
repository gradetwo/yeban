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
