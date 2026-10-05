//! **扩展工具的文本级守卫**（`ADR-0001` **D46** 判据 ① ② ③ 的"本机也能真跑"那一半）。
//!
//! ## 为什么要有它（而不是只靠运行期判据）
//!
//! `yeban-mcp` 传递依赖 `yeban-render` / `yeban-decode`（rayon / symphonia / rubato），
//! 因此**本机不能编译它**（`AGENTS.md` §5.2；`scripts/dev/heavy-deps.py` 会判它含重依赖，
//! `run-gates.sh crate yeban-mcp` 在本机直接 SKIP）。运行期判据（`tests/extension_tools.rs`）
//! 只能在 CI 上跑 —— 那么"本机做注入实验看它变红"就**做不到**，而那正是本仓库
//! 最看重的一类证据（`docs/ledger/mcp-core-notes.md` §4.4 的先例）。
//!
//! 处置：把三条**能从源码文本判定**的性质抽成纯函数（**只依赖 `std`**），于是
//!
//! ```text
//! rustc --edition 2024 --test -D warnings crates/yeban-mcp/src/domain/extension_audit.rs -o /tmp/x && /tmp/x
//! ```
//!
//! 在本机就能真跑；而运行期那三条判据仍然在 CI 上把它们再证明一遍（两侧都留下）。
//!
//! ## 三条性质
//!
//! | 守卫 | 性质 | 对应判据 | 注入（本机真做过） |
//! | :--- | :--- | :--- | :--- |
//! | [`scan_write_paths`] | 两个**写类**扩展工具的 `apply` 必须经过**唯一**提交入口 `undo_session::commit`，且不得直接改权威工程 | ① "写要落 `Op`、可逆" | 把 `commit` 换成 `op.apply(&mut active.project)` ⇒ 红 |
//! | [`scan_direct_edit_origins`] | 两个写类扩展工具提交的 `CommitRequest.origin` 必须是 `OpOrigin::McpEdit{agent_name: AGENT_NAME}`，且生产区不得再借 `AutomationRecord` / `Import` | ① "来源标签如实" | 把一处 `origin` 退回 `AutomationRecord` ⇒ 红 |
//! | [`scan_dry_run_entry_points`] | 每个扩展工具的**计划入口只拿共享引用**（`&Domain` / `&YebanProjectV1`） | ② "`dryRun` 结构上改不了状态" | 把 `&Domain` 改成 `&mut Domain` ⇒ 红 |
//! | [`scan_error_code_vocabulary`] | 实现写出的错误码集合 **==** 契约 enum（`ADR-0001` **D25** 的 20 值联集） | ③ "只用既有错误码，不发明新码" | 把一条 `as_str` 换成新码 ⇒ 红 |
//!
//! 四者的口径都**只**来自"源码文本 + 契约文件文本"，不复制任何运行期事实。

use std::collections::BTreeSet;

/// 写类扩展工具的含义：这两个文件里的 `apply` 一定会改权威工程。
pub const WRITE_PATH_FILES: [&str; 2] = ["domain/automation.rs", "domain/import_audio.rs"];

/// **唯一**的提交入口（`undo_session::commit` 是"改工程 + 写 Op 日志"的原子动作）。
pub const COMMIT_ENTRY: &str = "undo_session::commit(";

/// 直接改权威工程的形状（绕开提交 ⇒ Op 日志里没有这次变更 ⇒ 不可撤销）。
///
/// 只列**已知的**写法：这些是"看起来能工作、但撤销会坏"的那几种。判据
/// `a_direct_project_mutation_is_flagged` 逐个注入过。
pub const DIRECT_MUTATION_NEEDLES: [&str; 3] = [
    "apply(&mut active.project)",
    "apply(&mut project)",
    ".project =",
];

/// 计划入口的**精确签名**（`domain/mod.rs` 里的三条）。
///
/// 要求"精确签名存在"而不是"没有 `&mut`"：后者会被 `apply_inner(domain: &mut Domain)`
/// 误伤（那是**执行**入口，本来就该可变）。
pub const PLAN_ENTRY_SIGNATURES: [&str; 3] = [
    "fn plan_edit_automation(domain: &Domain, call: &ToolCall)",
    "fn plan_query_engine_state(domain: &Domain, call: &ToolCall)",
    "fn plan_import_audio(domain: &Domain, call: &ToolCall)",
];

/// 计划入口所在的文件（相对 `crates/yeban-mcp/src`）。
pub const PLAN_ENTRY_FILE: &str = "domain/mod.rs";

/// 计划入口的领域函数所处的两个文件（它们的 `plan` 只拿 `&YebanProjectV1`）。
pub const PLAN_LIBRARY_FILES: [&str; 2] = ["domain/automation.rs", "domain/import_audio.rs"];

/// 领域计划函数的签名片段（必须出现）。
pub const SHARED_PROJECT_REF: &str = "project: &YebanProjectV1";

/// 可变工程引用（**不得**出现在计划函数里）。
pub const MUTABLE_PROJECT_REF: &str = "&mut YebanProjectV1";

/// 直接编辑的**作者标签**（MCP 代理在活跃工程上直接改一位、**不**创建提案）。
pub const DIRECT_EDIT_ORIGIN: &str = "OpOrigin::McpEdit";

/// 作者名字段的精确形状（与 `UndoState.author` 同源：`AGENT_NAME`）。
pub const DIRECT_EDIT_AGENT: &str = "agent_name: super::AGENT_NAME";

/// 曾经被**误借**的来源变体（写类扩展工具的生产区里不得再出现）。
///
/// `AutomationRecord` = "自动化录制落盘"、`Import` = "外部工程/格式导入" ——
/// 两档都不是"代理直接改活跃工程"这件事。
pub const BORROWED_ORIGIN_NEEDLES: [&str; 2] = ["OpOrigin::AutomationRecord", "OpOrigin::Import"];

/// 取一份源码的**生产区**（`#[cfg(test)]` 属性行之前）。
///
/// ⚠ 与 `crate::undo_session::production_region` / `crate::domain::automation_audit::production_region`
/// 是**同一条口径**（整行开头匹配）。判据
/// `production_region_agrees_across_the_three_copies` 断言三份实现逐字节相同。
#[must_use]
pub fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 路径是否以某个后缀结尾（归一化分隔符；判据传进来的路径可能是绝对路径）。
#[must_use]
pub fn path_ends_with(path: &str, suffix: &str) -> bool {
    path.replace('\\', "/").ends_with(suffix)
}

/// 守卫 ①：两个写类扩展工具必须**经过唯一提交入口**，且不得直接改权威工程。
///
/// 返回违规清单（空 = 干净）；文件缺失也算违规（"找不到那个文件"不能当成"没问题"）。
#[must_use]
pub fn scan_write_paths(sources: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for suffix in WRITE_PATH_FILES {
        let Some((path, text)) = sources
            .iter()
            .find(|(path, _)| path_ends_with(path, suffix))
        else {
            violations.push(format!(
                "缺少写类扩展工具的源文件 `{suffix}` —— 守卫无法判定, 因此记违规"
            ));
            continue;
        };
        let production = production_region(text);
        if !production.contains(COMMIT_ENTRY) {
            violations.push(format!(
                "{path} 的生产区没有出现 `{COMMIT_ENTRY}` —— 写路径没有经过唯一提交入口 (Op 日志里不会有这次变更)"
            ));
        }
        for (lineno, line) in production.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            for needle in DIRECT_MUTATION_NEEDLES {
                if line.contains(needle) {
                    violations.push(format!(
                        "{path}:{} 直接改权威工程 (`{needle}`) —— 必须走 `{COMMIT_ENTRY}` \
                         才能同时写进 Op 日志 (否则撤销坏掉): {code}",
                        lineno + 1
                    ));
                }
            }
        }
    }
    violations
}

/// 守卫 ④：两个**写类**扩展工具提交的 `CommitRequest.origin` 必须是 `McpEdit`。
///
/// 为什么单列一条：**来源标签不准**是这一族的原始缺陷 —— MCP 的直接编辑一度借用
/// "自动化录制落盘"（`AutomationRecord`）与"外部导入"（`Import`），于是审计看到的
/// 作者不是"代理直接改活跃工程"。这条守卫把两件事钉死：
///
/// - 每个 `origin:` 的右值必须以 [`DIRECT_EDIT_ORIGIN`] 开头（且带上
///   [`DIRECT_EDIT_AGENT`]，与 `UndoState.author` 同源）；
/// - 生产区（注释行除外）里不得再出现 [`BORROWED_ORIGIN_NEEDLES`] 里的变体。
///
/// 返回违规清单（空 = 干净）；文件缺失也算违规（"找不到那个文件"不能当成"没问题"）。
#[must_use]
pub fn scan_direct_edit_origins(sources: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    for suffix in WRITE_PATH_FILES {
        let Some((path, text)) = sources
            .iter()
            .find(|(path, _)| path_ends_with(path, suffix))
        else {
            violations.push(format!(
                "缺少写类扩展工具的源文件 `{suffix}` —— 守卫无法判定, 因此记违规"
            ));
            continue;
        };
        let production = production_region(text);
        let mut seen = 0_usize;
        for (lineno, line) in production.lines().enumerate() {
            if line.trim_start().starts_with("//") {
                continue;
            }
            let Some((_, rhs)) = line.split_once("origin:") else {
                continue;
            };
            seen += 1;
            let rhs = rhs.trim();
            if !rhs.starts_with(DIRECT_EDIT_ORIGIN) {
                violations.push(format!(
                    "{path}:{} 直接编辑的来源标签必须是 `{DIRECT_EDIT_ORIGIN}`, 实际是 `{rhs}` \
                     —— 借 `AutomationRecord`/`Import` 等于把'代理直接改活跃工程'记成别的动作",
                    lineno + 1
                ));
            }
        }
        if seen == 0 {
            violations.push(format!(
                "{path} 的生产区没有 `origin:` —— 写路径的作者标签无处可查"
            ));
        }
        if !production.contains(DIRECT_EDIT_AGENT) {
            violations.push(format!(
                "{path} 的生产区没有 `{DIRECT_EDIT_AGENT}` —— 直接编辑的作者名必须来自 `AGENT_NAME`"
            ));
        }
        for needle in BORROWED_ORIGIN_NEEDLES {
            for (lineno, line) in production.lines().enumerate() {
                if line.trim_start().starts_with("//") {
                    continue;
                }
                if line.contains(needle) {
                    violations.push(format!(
                        "{path}:{} 仍在借 `{needle}` —— 那不是'代理直接编辑'这一档: {}",
                        lineno + 1,
                        line.trim()
                    ));
                }
            }
        }
    }
    violations
}

/// 守卫 ②：扩展工具的**计划入口只拿共享引用**（`dryRun` 的结构性保证）。
#[must_use]
pub fn scan_dry_run_entry_points(sources: &[(String, String)]) -> Vec<String> {
    let mut violations = Vec::new();
    let Some((path, text)) = sources
        .iter()
        .find(|(path, _)| path_ends_with(path, PLAN_ENTRY_FILE))
    else {
        return vec![format!("缺少 `{PLAN_ENTRY_FILE}` —— 守卫无法判定")];
    };
    let production = production_region(text);
    for signature in PLAN_ENTRY_SIGNATURES {
        if !production.contains(signature) {
            violations.push(format!(
                "{path} 里找不到计划入口的共享引用签名 `{signature}` —— \
                 dryRun 的结构性保证 (借用检查器) 被削弱了"
            ));
        }
    }
    // 可变领域引用直接点名（比"签名找不到"更好读：它指到行）。
    for (lineno, line) in production.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        if line.contains("fn plan_") && line.contains("&mut Domain") {
            violations.push(format!(
                "{path}:{} 计划入口拿到了**可变**领域引用 —— dryRun 因此能改状态: {code}",
                lineno + 1
            ));
        }
    }
    for suffix in PLAN_LIBRARY_FILES {
        let Some((path, text)) = sources
            .iter()
            .find(|(path, _)| path_ends_with(path, suffix))
        else {
            violations.push(format!("缺少 `{suffix}` —— 守卫无法判定"));
            continue;
        };
        let production = production_region(text);
        if !production.contains(SHARED_PROJECT_REF) {
            violations.push(format!(
                "{path} 的生产区没有 `{SHARED_PROJECT_REF}` —— 计划函数不再只拿共享引用"
            ));
        }
        for (lineno, line) in production.lines().enumerate() {
            if line.contains(MUTABLE_PROJECT_REF) {
                violations.push(format!(
                    "{path}:{} 计划阶段拿到了可变工程 (`{MUTABLE_PROJECT_REF}`): {}",
                    lineno + 1,
                    line.trim_start()
                ));
            }
        }
    }
    violations
}

/// 从契约文件里提取 `ToolResponse.error.code` 的 enum（**唯一权威**）。
///
/// 定位方式：`"code"` → 其后的 `"enum"` → 其后的第一个 `[ … ]`；方括号里的
/// 双引号字符串就是错误码清单。提取不出来时返回 `None`（调用方记违规 —— "读不懂契约"
/// 绝不能当成"契约没问题"）。
#[must_use]
pub fn parse_contract_error_codes(schema_text: &str) -> Option<Vec<String>> {
    let code_at = schema_text.find("\"code\"")?;
    let rest = &schema_text[code_at..];
    let enum_at = rest.find("\"enum\"")?;
    let rest = &rest[enum_at..];
    let open = rest.find('[')?;
    let close = rest[open..].find(']')? + open;
    let codes = quoted_strings(&rest[open + 1..close]);
    (!codes.is_empty()).then_some(codes)
}

/// 一段文本里所有双引号字符串（按出现顺序；不处理转义 —— 错误码是纯 ASCII 大写）。
#[must_use]
pub fn quoted_strings(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut inside = false;
    for character in text.chars() {
        match character {
            '"' if inside => {
                out.push(std::mem::take(&mut current));
                inside = false;
            }
            '"' => inside = true,
            other if inside => current.push(other),
            _ => {}
        }
    }
    out
}

/// `ErrorCode::as_str` 的 `Self::Variant => "CODE"` 映射（从源码文本里读出来）。
#[must_use]
pub fn parse_error_code_strings(tools_source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let production = production_region(tools_source);
    for line in production.lines() {
        let code = line.trim();
        if !code.starts_with("Self::") || !code.contains("=>") {
            continue;
        }
        let Some(variant) = code
            .strip_prefix("Self::")
            .and_then(|rest| rest.split_whitespace().next())
        else {
            continue;
        };
        let Some(text) = code.split("=>").nth(1) else {
            continue;
        };
        let strings = quoted_strings(text);
        if let Some(first) = strings.first() {
            out.push((variant.to_owned(), first.clone()));
        }
    }
    out
}

/// `pub const <NAME>: [Self; N] = [ Self::A, Self::B, … ];` 里的变体名。
#[must_use]
pub fn parse_self_array(tools_source: &str, constant: &str) -> Vec<String> {
    let production = production_region(tools_source);
    let Some(start) = production.find(constant) else {
        return Vec::new();
    };
    let rest = &production[start..];
    let Some(open) = rest.find('[').and_then(|first| {
        // 第一个 `[` 属于类型注解 `[Self; 20]`，第二个才是数组字面量。
        rest[first + 1..].find('[').map(|second| first + 1 + second)
    }) else {
        return Vec::new();
    };
    // ⚠ `close` 必须换算回 `rest` 的下标（`rest[open..]` 里的下标是相对的）。
    let Some(close) = rest[open..].find(']').map(|close| close + open) else {
        return Vec::new();
    };
    let body = &rest[open + 1..close];
    body.split("Self::")
        .skip(1)
        .filter_map(|chunk| {
            let name: String = chunk
                .chars()
                .take_while(|character| character.is_ascii_alphanumeric() || *character == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

/// 守卫 ③：实现写出的错误码集合 **==** 契约 enum（`ADR-0001` **D25**）。
///
/// 口径：`ErrorCode::SCHEMA_CONTRACT`（实现侧声明参与契约的集合）经 `as_str` 映射成
/// 字符串之后，必须与契约文件里 `ToolResponse.error.code.enum` **集合相等**：
/// 少一个（契约扩了、实现没跟上）或多一个（**发明了新码**）都违规。
#[must_use]
pub fn scan_error_code_vocabulary(sources: &[(String, String)], schema_text: &str) -> Vec<String> {
    let mut violations = Vec::new();
    let Some(contract) = parse_contract_error_codes(schema_text) else {
        return vec!["无法从契约文件里提取 ToolResponse.error.code.enum".to_owned()];
    };
    let Some((path, tools_source)) = sources
        .iter()
        .find(|(path, _)| path_ends_with(path, "src/tools.rs"))
    else {
        return vec!["缺少 `src/tools.rs` —— 守卫无法判定".to_owned()];
    };
    let mapping = parse_error_code_strings(tools_source);
    if mapping.is_empty() {
        return vec![format!("{path} 里读不出 `ErrorCode::as_str` 的映射")];
    }
    let declared = parse_self_array(tools_source, "pub const SCHEMA_CONTRACT");
    if declared.is_empty() {
        return vec![format!("{path} 里读不出 `SCHEMA_CONTRACT` 的变体清单")];
    }
    let implemented: BTreeSet<String> = declared
        .iter()
        .filter_map(|variant| {
            mapping
                .iter()
                .find(|(name, _)| name == variant)
                .map(|(_, code)| code.clone())
        })
        .collect();
    if implemented.len() != declared.len() {
        violations.push(format!(
            "{path}: `SCHEMA_CONTRACT` 里有变体没有 `as_str` 映射 (实得 {implemented:?})"
        ));
    }
    let contract_set: BTreeSet<String> = contract.iter().cloned().collect();
    for missing in contract_set.difference(&implemented) {
        violations.push(format!(
            "契约 enum 里的 `{missing}` 没有被实现覆盖 (SCHEMA_CONTRACT 缺它)"
        ));
    }
    for invented in implemented.difference(&contract_set) {
        violations.push(format!(
            "实现写出了契约 enum 之外的错误码 `{invented}` —— ADR-0001 D25 不许发明新码"
        ));
    }
    violations
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sources(entries: &[(&str, &str)]) -> Vec<(String, String)> {
        entries
            .iter()
            .map(|(path, text)| ((*path).to_owned(), (*text).to_owned()))
            .collect()
    }

    /// 一份**最小但结构真实**的 tools.rs 片段（够 `parse_self_array` / `as_str` 用）。
    fn fake_tools_source(codes: &[(&str, &str)]) -> String {
        let mut as_str = String::from(
            "impl ErrorCode {\n    pub const fn as_str(self) -> &'static str {\n        match self {\n",
        );
        for (variant, code) in codes {
            as_str.push_str(&format!("            Self::{variant} => \"{code}\",\n"));
        }
        as_str.push_str("        }\n    }\n");
        let mut array = String::from("    pub const SCHEMA_CONTRACT: [Self; N] = [\n");
        for (variant, _) in codes {
            array.push_str(&format!("        Self::{variant},\n"));
        }
        array.push_str("    ];\n}\n");
        format!("{as_str}{array}")
    }

    fn fake_schema(codes: &[&str]) -> String {
        let list = codes
            .iter()
            .map(|code| format!("\"{code}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "{{\n  \"properties\": {{\n    \"error\": {{\n      \"properties\": {{\n        \"code\": {{\n          \"enum\": [{list}]\n        }}\n      }}\n    }}\n  }}\n}}\n"
        )
    }

    #[test]
    fn the_write_path_guard_requires_the_commit_entry() {
        let clean = sources(&[
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "fn apply() {\n    undo_session::commit(graph, &mut active.project, undo, request)?;\n}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "fn apply() {\n    undo_session::commit(graph, &mut active.project, undo, request)?;\n}\n",
            ),
        ]);
        assert_eq!(scan_write_paths(&clean), Vec::<String>::new());

        // 注入 1：把提交换成"只改内存不落 Op"。
        let injected = sources(&[
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "fn apply(write: &Op) {\n    write.op.apply(&mut active.project).expect(\"x\");\n}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "fn apply() {\n    undo_session::commit(graph, &mut active.project, undo, request)?;\n}\n",
            ),
        ]);
        let found = scan_write_paths(&injected);
        assert!(
            found.iter().any(|line| line.contains("没有出现")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|line| line.contains("apply(&mut active.project)")),
            "{found:?}"
        );
    }

    #[test]
    fn a_missing_write_file_is_a_violation_not_a_pass() {
        let found = scan_write_paths(&sources(&[]));
        assert_eq!(found.len(), WRITE_PATH_FILES.len(), "{found:?}");
    }

    /// 守卫 ④：直接编辑的来源标签必须是 `McpEdit`，且不得再借那两个变体。
    #[test]
    fn the_direct_edit_guard_requires_the_mcp_edit_label() {
        let clean = sources(&[
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "fn apply() {\n    let request = CommitRequest {\n        \
                 origin: OpOrigin::McpEdit { agent_name: super::AGENT_NAME.to_owned() },\n    };\n}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "fn apply() {\n    let request = CommitRequest {\n        \
                 origin: OpOrigin::McpEdit { agent_name: super::AGENT_NAME.to_owned() },\n    };\n}\n",
            ),
        ]);
        assert_eq!(scan_direct_edit_origins(&clean), Vec::<String>::new());

        // 注入 3：一个站点退回被借用的变体（这正是本线要修的原始缺陷）。
        let injected = sources(&[
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "fn apply() {\n    let request = CommitRequest {\n        \
                 origin: OpOrigin::AutomationRecord,\n    };\n}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "fn apply() {\n    let request = CommitRequest {\n        \
                 origin: OpOrigin::McpEdit { agent_name: super::AGENT_NAME.to_owned() },\n    };\n}\n",
            ),
        ]);
        let found = scan_direct_edit_origins(&injected);
        assert!(
            found.iter().any(|line| line.contains("必须是")),
            "注入必须被'标签不对'那条抓住: {found:?}"
        );
        assert!(
            found
                .iter()
                .any(|line| line.contains("OpOrigin::AutomationRecord")),
            "注入必须被'仍在借'那条抓住: {found:?}"
        );
    }

    /// 缺文件时必须记违规（否则守卫在文件被改名后**静默变成空转**）。
    #[test]
    fn a_write_file_without_an_origin_is_a_violation_not_a_pass() {
        let found = scan_direct_edit_origins(&sources(&[]));
        assert_eq!(found.len(), WRITE_PATH_FILES.len(), "{found:?}");
        assert!(found.iter().all(|line| line.contains("缺少")), "{found:?}");

        // 有文件但**没有** `origin:` ⇒ 也必须红（否则"标签对不对"无从判定）。
        let no_origin = sources(&[
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "fn apply() {\n    let request = CommitRequest { now_ms };\n}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "fn apply() {\n    let request = CommitRequest { now_ms };\n}\n",
            ),
        ]);
        let found = scan_direct_edit_origins(&no_origin);
        assert_eq!(found.len(), 2 * WRITE_PATH_FILES.len(), "{found:?}");
    }

    #[test]
    fn the_dry_run_guard_requires_shared_references_only() {
        let clean = sources(&[
            (
                "crates/yeban-mcp/src/domain/mod.rs",
                "fn plan_edit_automation(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n\
                 fn plan_query_engine_state(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n\
                 fn plan_import_audio(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "pub fn plan(project: &YebanProjectV1) {}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "pub fn plan(project: &YebanProjectV1) {}\n",
            ),
        ]);
        assert_eq!(scan_dry_run_entry_points(&clean), Vec::<String>::new());

        // 注入 2：计划入口拿到可变引用（dryRun 于是**能**改状态）。
        let injected = sources(&[
            (
                "crates/yeban-mcp/src/domain/mod.rs",
                "fn plan_edit_automation(domain: &mut Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n\
                 fn plan_query_engine_state(domain: &Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n\
                 fn plan_import_audio(domain: &mut Domain, call: &ToolCall) -> Result<Plan, Fault> {}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/automation.rs",
                "pub fn plan(project: &mut YebanProjectV1) {}\n",
            ),
            (
                "crates/yeban-mcp/src/domain/import_audio.rs",
                "pub fn plan(project: &YebanProjectV1) {}\n",
            ),
        ]);
        let found = scan_dry_run_entry_points(&injected);
        // 违规清单：mod.rs 的两条入口签名消失 + 两处可变引用点名（edit / import），
        // automation.rs 少了共享引用 + 一处可变引用。query 那条仍然绿（它没被注入）。
        assert_eq!(found.len(), 6, "{found:?}");
        assert!(
            found.iter().any(|line| line.contains("&mut Domain")),
            "{found:?}"
        );
        assert!(
            found
                .iter()
                .any(|line| line.contains("&mut YebanProjectV1")),
            "{found:?}"
        );
    }

    #[test]
    fn the_error_code_guard_compares_the_implementation_against_the_contract() {
        let codes = [("IoError", "IO_ERROR"), ("Busy", "BUSY")];
        let source = fake_tools_source(&codes);
        let batch = sources(&[("crates/yeban-mcp/src/tools.rs", source.as_str())]);
        let contract = fake_schema(&["BUSY", "IO_ERROR"]);
        assert_eq!(
            scan_error_code_vocabulary(&batch, &contract),
            Vec::<String>::new()
        );

        // 注入 3：实现写出一个契约里没有的码（"发明新码"）。
        let injected_codes = [
            ("IoError", "IO_ERROR"),
            ("Busy", "BUSY"),
            ("ThemeChanged", "THEME_CHANGED"),
        ];
        let injected_source = fake_tools_source(&injected_codes);
        let injected = sources(&[("crates/yeban-mcp/src/tools.rs", injected_source.as_str())]);
        let found = scan_error_code_vocabulary(&injected, &contract);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("THEME_CHANGED"), "{found:?}");

        // 反向：契约扩了而实现没跟上（少一个）也要红。
        let wider = fake_schema(&["BUSY", "IO_ERROR", "DISK_FULL"]);
        let found = scan_error_code_vocabulary(&batch, &wider);
        assert_eq!(found.len(), 1, "{found:?}");
        assert!(found[0].contains("DISK_FULL"), "{found:?}");
    }

    #[test]
    fn unreadable_contract_or_missing_tools_source_is_a_violation() {
        let batch = sources(&[("crates/yeban-mcp/src/tools.rs", "fn x() {}")]);
        let found = scan_error_code_vocabulary(&batch, "{}");
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("提取"), "{found:?}");
        let found = scan_error_code_vocabulary(&sources(&[]), &fake_schema(&["BUSY"]));
        assert_eq!(found.len(), 1);
        assert!(found[0].contains("tools.rs"), "{found:?}");
    }

    #[test]
    fn the_parsers_read_the_real_shapes() {
        let source = fake_tools_source(&[("ProjectLocked", "PROJECT_LOCKED"), ("Busy", "BUSY")]);
        assert_eq!(
            parse_error_code_strings(&source),
            vec![
                ("ProjectLocked".to_owned(), "PROJECT_LOCKED".to_owned()),
                ("Busy".to_owned(), "BUSY".to_owned()),
            ]
        );
        assert_eq!(
            parse_self_array(&source, "pub const SCHEMA_CONTRACT"),
            vec!["ProjectLocked".to_owned(), "Busy".to_owned()]
        );
        assert_eq!(
            parse_contract_error_codes(&fake_schema(&["A", "B"])),
            Some(vec!["A".to_owned(), "B".to_owned()])
        );
        assert_eq!(quoted_strings("x \"one\" y \"two\" z"), vec!["one", "two"]);
    }

    #[test]
    fn production_region_and_path_matching_are_the_shared_conventions() {
        let text = "fn f() {}\n#[cfg(test)]\nmod tests {}\n";
        assert!(production_region(text).contains("fn f"));
        assert!(!production_region(text).contains("mod tests"));
        assert!(path_ends_with(
            "/abs/crates/yeban-mcp/src/domain/mod.rs",
            "domain/mod.rs"
        ));
        assert!(path_ends_with(
            "crates\\yeban-mcp\\src\\domain\\mod.rs",
            "domain/mod.rs"
        ));
        assert!(!path_ends_with(
            "crates/yeban-app/src/domain/mod.rs",
            "yeban-mcp/src/domain/mod.rs"
        ));
    }
}
