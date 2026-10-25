//! **`Op` 变体的第三口径**：工具面可达性（源码级普查）+ `ops[].kind` 的双射守卫。
//!
//! ## 三个口径的区别（本文件只做第三口径）
//!
//! | 口径 | 问题 | 弱点 |
//! | :--- | :--- | :--- |
//! | ① 构造点 | `src` 里有没有 `Op::V {`？ | 含 **match 模式**，因此是宽松下限 |
//! | ② 任何提及 | `src` 里有没有提到 `Op::V`？ | 含**文档注释** |
//! | ③ 工具面可达 | **哪个工具的产线真的构造它**？ | 必须先分辨"构造"与"模式" |
//!
//! 本文件把 ③ 变成机制：对**工具实现模块**的生产区扫构造点，并要求
//! 每个 `Op` 变体在**登记的模块**里至少有一个**构造**（不是模式）。
//!
//! ## 怎么分辨"构造"与"模式"（机械规则，逐条给理由）
//!
//! 1. `matches!(x, Op::V { .. })` ⇒ 模式（回溯同一行找未被闭合的 `matches!(`）；
//! 2. 花括号配对之后的第一个非空白是 `=>` ⇒ 模式（match 臂）；
//! 3. `Op::V` 前面（跳过空白）是 `|` ⇒ 模式（或模式交替）、其余一律算构造。
//!
//! 规则②用**花括号配对**（不是看行尾），因此跨行的变体体也能判对。
//!
//! ## 为什么"登记的模块"是一张表而不是"任一模块"
//!
//! 只要求"某个模块有构造点"时，把 `import_audio` 里的构造点删掉**不会**红
//! （`section_build` 里还有一份）—— 那是"两条互为冗余的防线"同一类问题。
//! 本表逐**变体 × 模块**登记，因此删掉任何一处登记的构造点都会红。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use yeban_mcp::domain::notes::OP_KINDS;
use yeban_mcp::undo_session::read_rust_sources;

// ⚠ 这里**曾经**有一个 `TOOL_MODULES: [&str; 7]`（"只在列出的文件里扫构造点"）。
// 它**从未被引用**，而且**已经过期**：真值 9 个模块，清单只列 7 个 ——
// 缺 `domain/proposal.rs`（`Op::SetMacro`）与 `domain/section.rs`（`Op::Batch`）。
// 本文件的 `sources()` 实际扫的是**整个 `src`**，所以那个常量既不生效、又会**误导后人**
// 以为存在一份"工具实现模块"的权威清单。⇒ **删掉它，而不是修它**：没有对账守卫的
// 手抄清单就是债（台账 R427／第 44 条）；真要一份清单，就该**从 `src` 现读**。

/// 黄金表：每个 `Op` 变体**必须**在哪些工具模块里有一个构造点。
///
/// 读法：删掉任一处登记的构造点 ⇒ 本判据红。
pub const EXPECTED_SITES: [(&str, &[&str]); 49] = [
    ("AddNote", &["domain/notes.rs"]),
    ("DeleteNote", &["domain/notes.rs"]),
    ("MoveNote", &["domain/notes.rs"]),
    ("ModifyNoteVelocity", &["domain/notes.rs"]),
    // ⭐ 本线新增：`MoveNote` 只平移起点、时值没有字段级通路 ⇒ `setNoteTiming`
    // 的 `parse_one` / `compile` 在 `domain/notes.rs` 构造 `Op::SetNoteTiming`。
    ("SetNoteTiming", &["domain/notes.rs"]),
    // ⭐ `#329`：三个表现力字段此前只在 `add` 时能给 ⇒ `setNoteExpression` 的
    // `parse_one` / `compile` 在 `domain/notes.rs` 构造 `Op::SetNoteExpression`。
    ("SetNoteExpression", &["domain/notes.rs"]),
    // ⭐ `#334`：弯音曲线 / 滑音此前只在 `add` 时能给（改形状只能删 + 加，会换身份）
    // ⇒ `setNotePitchShape` 的 `parse_one` / `compile` 构造 `Op::SetNotePitchShape`。
    ("SetNotePitchShape", &["domain/notes.rs"]),
    (
        "AddClip",
        &[
            "domain/notes.rs",
            "domain/import_audio.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
    ("RemoveClip", &["domain/notes.rs"]),
    (
        "AddRoutingNode",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("RemoveRoutingNode", &["domain/notes.rs"]),
    (
        "AddClipPlacement",
        &[
            "domain/mod.rs",
            "domain/import_audio.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
    ("RemoveClipPlacement", &["domain/mod.rs"]),
    ("MoveClipPlacement", &["domain/mod.rs"]),
    (
        "AddTrack",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("RemoveTrack", &["domain/notes.rs"]),
    (
        "ConnectRouting",
        &["domain/project_create.rs", "domain/section_build.rs"],
    ),
    ("DisconnectRouting", &["domain/notes.rs"]),
    ("SetTempo", &["domain/notes.rs"]),
    ("SetAuthor", &["domain/notes.rs"]),
    ("SetTitle", &["domain/notes.rs"]),
    ("SetTrackName", &["domain/notes.rs"]),
    ("SetClipName", &["domain/notes.rs"]),
    ("SetClipGain", &["domain/notes.rs"]),
    ("SetDeviceName", &["domain/notes.rs"]),
    ("SetDeviceBypassed", &["domain/notes.rs"]),
    ("SetClipPlacementMuted", &["domain/notes.rs"]),
    ("SetTrackColor", &["domain/notes.rs"]),
    ("SetClipPlacementDuration", &["domain/notes.rs"]),
    ("SetRoutingGain", &["domain/notes.rs"]),
    ("InsertDevice", &["domain/notes.rs"]),
    ("RemoveDevice", &["domain/notes.rs"]),
    ("SetParam", &["domain/notes.rs"]),
    ("SetTrackMute", &["domain/notes.rs"]),
    ("SetTrackSolo", &["domain/notes.rs"]),
    ("SetMacro", &["domain/macros.rs"]),
    (
        "SetAutomationPoint",
        &["domain/automation.rs", "domain/macros.rs"],
    ),
    ("RemoveAutomationPoint", &["domain/notes.rs"]),
    ("SetAutomationLane", &["domain/notes.rs"]),
    ("RemoveAutomationLane", &["domain/notes.rs"]),
    ("SetSection", &["domain/section_build.rs"]),
    ("RemoveSection", &["domain/notes.rs"]),
    // ⭐ 步 ② 落地：`yeban_import_audio` 在同一个 `Op::Batch` 里构造它
    ("SetAsset", &["domain/import_audio.rs"]),
    ("RemoveAsset", &["domain/notes.rs"]),
    ("SetScene", &["domain/notes.rs"]),
    ("RemoveScene", &["domain/notes.rs"]),
    (
        "Batch",
        &[
            "domain/mod.rs",
            "domain/macros.rs",
            "domain/project_create.rs",
            "domain/section_build.rs",
        ],
    ),
    // ⚠ 本轮补登记：它在 `domain/notes.rs` 的 `compile` 臂有构造点，
    // 此前**只是漏登记**（由 `mod-mcp` 实测发现）。见本文件新增的反向断言。
    ("SetTransportConfig", &["domain/notes.rs"]),
    // ⭐ `#221`：`SetTrackSampler` 从下面的 `KNOWN_UNREACHABLE` **移到这里** ——
    // `domain/notes.rs` 的 `parse_one` / `compile` 现在有它的构造点
    // （`NoteOp::SetTrackSampler` ⇒ `Op::SetTrackSampler`），工具面因此**可达**。
    ("SetTrackSampler", &["domain/notes.rs"]),
];

/// **模型里有、而工具面**确实**没有构造点**的 `Op` 变体（显式欠账清单）。
///
/// ⭐ 为什么要有它、而不是"把 `EXPECTED_SITES` 补齐到模型全集"：本仓的口径是
/// **欠账要显式**（与 `PENDING_CONTRACT_OPS` 同型）—— 一个变体若真的不可达，
/// 就要**如实登记**，⛔ 不许为了过判据而伪造一个构造点。
///
/// ⚠ 每一条都必须带理由；清空它意味着"工具面覆盖了模型全集"。
///
/// ⭐ `#221`：本清单**重新清空** —— 最后一条欠账 `SetTrackSampler` 还完了
/// （`yeban_edit_notes` 的 `setTrackSampler` 有 `parse_one` / `compile` 构造点，
/// 已移进上面的 `EXPECTED_SITES`）。判据仍双向守着：谁把一条**可达**的变体塞回这里，
/// 或把一条**不可达**的变体写进 `EXPECTED_SITES`，都会红。
const KNOWN_UNREACHABLE: [(&str, &str); 0] = [
    // ⭐ 历史（不再有活条目，保留形状让下一条欠账有地方登记）：
    // - `#77` 登记 `SetTrackSampler`（"工具面还没有采样器绑定通路"）；
    // - `#221` 还清它 —— 工具面现在有通路，因此这条理由**已经不成立**，必须删掉。
    //
    // ⭐ 更早还清过的四条：`SetTempo` / `SetAuthor` / `SetAsset`（步 ②）/ `RemoveAsset`（步 ③）。
];
// ⭐ 历史: 本清单的清空**是证明过的**（临时加回一条 ⇒ 反向断言逐字报"`SetTempo` 既在
// `KNOWN_UNREACHABLE` 又在 `EXPECTED_SITES` ⇒ 两处矛盾，请二选一" ⇒ 然后恢复）。

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn production_region(text: &str) -> String {
    text.lines()
        .take_while(|line| !line.trim_start().starts_with("#[cfg(test)]"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 该文本里**被构造**的 `Op` 变体名（已剔除 match 模式）。
/// **全文区间掩码**（R94）：注释区间与字符串**内部**的字节下标为 `true`。
///
/// * 字符串的**开/闭引号不掩** —— 否则针自身的引号会被判成"在串里"，扫描器就再也读不到东西
///   （本文件第五批实测：把引号一起掩掉 ⇒ reason 数变成 0）。
/// * **换行永不掩**（⛔ 否则行号与行结构都会漂移）。
/// * 只标区间、**不删字符**（⛔ 不用"抹掉内容"代替"跳过区间"：那会改变表达式语义 ⇒ 假红）。
fn skipped_offsets(text: &str) -> Vec<bool> {
    let bytes = text.as_bytes();
    let mut mask = vec![false; bytes.len()];
    let mut index = 0usize;
    let mut in_string = false;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_string {
            if byte == b'\\' {
                if index + 1 < bytes.len() {
                    mask[index + 1] = true;
                }
                index += 2;
                continue;
            }
            if byte == b'"' {
                in_string = false;
                index += 1;
                continue;
            }
            if byte != b'\n' {
                mask[index] = true;
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
                mask[index] = true;
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    mask
}

/// 该字节是否落在注释／字符串**内部**（越界按"不在"处理）。
fn is_skipped(mask: &[bool], at: usize) -> bool {
    mask.get(at).copied().unwrap_or(false)
}

fn constructed_variants(text: &str) -> BTreeSet<String> {
    let bytes = text.as_bytes();
    // R94：先算区间掩码 —— **注释与字符串里的 `Op::X {` 不是构造点**
    // （本函数此前完全没有掩码：注释里写一个 `Op::SetSection {{` 就会被算进去）。
    let mask = skipped_offsets(text);
    let mut out = BTreeSet::new();
    let mut index = 0usize;
    while let Some(offset) = text[index..].find("Op::") {
        let at = index + offset;
        index = at + 4;
        if is_skipped(&mask, at) {
            continue;
        }
        // 排除 `NoteOp::`：前一个字符不得是标识符字符
        if at > 0 {
            let previous = bytes[at - 1];
            if previous.is_ascii_alphanumeric() || previous == b'_' {
                continue;
            }
        }
        // 变体名
        let name_start = at + 4;
        let name_end = text[name_start..]
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .map_or(text.len(), |skip| name_start + skip);
        let name = &text[name_start..name_end];
        if name.is_empty() {
            continue;
        }
        // 变体后面（跳过空白）必须是 `{`；否则是单元变体（`Op::RemoveClip` 之类）⇒ 构造
        let mut cursor = name_end;
        while cursor < bytes.len() && bytes[cursor].is_ascii_whitespace() {
            cursor += 1;
        }
        if cursor >= bytes.len() || bytes[cursor] != b'{' {
            out.insert(name.to_owned());
            continue;
        }
        // 花括号配对
        let mut depth = 0i32;
        let mut end = cursor;
        while end < bytes.len() {
            match bytes[end] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            end += 1;
        }
        let mut after = end + 1;
        while after < bytes.len() && bytes[after].is_ascii_whitespace() {
            after += 1;
        }
        // 规则②：紧跟 `=>` ⇒ match 臂模式
        if text[after..].starts_with("=>") {
            continue;
        }
        // 规则④：紧跟 `|` ⇒ 模式（模式交替的**左边**那一支）。
        // 与规则③（`Op::V` 前面是 `|` ⇒ 交替的右边那一支）配对：交替要两支都判成模式
        // 才算完整。两条**各自**可独立观测 —— 见自测里的 `if let A | B = op {}` 输入
        // （③ 抓 B、④ 抓 A）。实测教训：只用旧输入（含 `matches!` 与 `=>`）时，
        // 去掉规则③**全绿** ⇒ 那两条当时互为冗余。
        if text[after..].starts_with('|') {
            continue;
        }
        // 规则①：同一行里前面有未闭合的 `matches!(` ⇒ 模式
        let line_start = text[..at].rfind('\n').map_or(0, |position| position + 1);
        let prefix = &text[line_start..at];
        if let Some(call) = prefix.rfind("matches!(")
            && !prefix[call..].contains(')')
        {
            continue;
        }
        // 规则③：前面（跳过空白）是 `|` ⇒ 模式交替
        let trimmed = prefix.trim_end();
        if trimmed.ends_with('|') {
            continue;
        }
        out.insert(name.to_owned());
    }
    out
}

/// 该文本里 `parse_one` 的 match 臂认得的 `kind`（把常量名解析成字面量）。
///
/// ## ⚠ 找不到锚点必须**响亮失败**，⛔ 不许静默返回空集（第 41 条）
///
/// 这里**曾经**是两处 `return BTreeSet::new()`。⚠ 消费判据
/// [`the_catalog_kinds_are_exactly_the_parse_arms`] 做的是**双向差集** ⇒ 抽到空集时
/// 它**仍会红**（⛔ 不假绿），⭐ 但报文是「只在目录里（没有解析臂）」——
/// ⭐ **把「抽取器瞎了」报成「目录写错了」**，会把维护者引去改 `OP_KINDS`。
/// ⭐ 即"红 ≠ 诊断正确" ⇒ 兜底必须是**防空**、不是**对账**。
///
/// 因此两处改 `.expect(...)`，并在报文里写明**找不到哪个锚点**、
/// 且**指名是抽取规则失效（⛔ 不是目录的问题）**。
///
/// ⚠ 入口先**规范化换行**（与 `project.rs` 的 `routing_graph_fields_in_source`
/// 同一条 R51 纪律）：CI 的 windows 腿检出的是 **CRLF**，而本函数按**文本锚点**
/// 切片（`find("\n}\n")`）—— 不规范化时 Windows 上找不到锚点。
fn kinds_in_parse_one(text: &str) -> BTreeSet<String> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    // 先建常量表：`pub const X_KIND: &str = "literal";`
    let mut constants: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("pub const ")
            && let Some((name, tail)) = rest.split_once(':')
            && let Some(value) = tail.split('"').nth(1)
        {
            constants.insert(name.trim().to_owned(), value.to_owned());
        }
    }
    let start = text.find("fn parse_one(").expect(
        "⭐ 抽取规则失效（⛔ 不是目录的问题）: 在 notes.rs 里找不到锚点 `fn parse_one(`。\
         若 `parse_one` 被改名/移位, 请**同时**改本函数与 `OP_KINDS` 的对账口径 —— \
         ⛔ 不要靠「抽到空集」来兜底（那会把本判据的报文误导成「目录写错了」）",
    );
    let body = &text[start..];
    let end = body.find("\n}\n").expect(
        "⭐ 抽取规则失效（⛔ 不是目录的问题）: 从 `fn parse_one(` 起找不到结束锚点 `\n}\n`。\
         若 `parse_one` 的函数体形状变了（例如尾随注释/换行风格变了）, 请改本函数 —— \
         ⛔ 不要靠「抽到空集」来兜底",
    );
    let body = &body[..end];
    let mut out = BTreeSet::new();
    for line in body.lines() {
        let trimmed = line.trim_start();
        let Some((lhs, _rhs)) = trimmed.split_once("=>") else {
            continue;
        };
        let lhs = lhs.trim();
        if let Some(literal) = lhs
            .strip_prefix('"')
            .and_then(|rest| rest.split('"').next())
        {
            out.insert(literal.to_owned());
        } else if let Some(value) = constants.get(lhs) {
            out.insert(value.clone());
        }
    }
    out
}

fn sources() -> Vec<(String, String)> {
    read_rust_sources(&[manifest_dir().join("src")])
}

fn module_text(files: &[(String, String)], suffix: &str) -> Option<String> {
    files
        .iter()
        .find(|(path, _text)| path.replace('\\', "/").ends_with(suffix))
        .map(|(_path, text)| production_region(text))
}

#[test]
fn every_op_variant_has_a_registered_construction_site() {
    let files = sources();
    assert!(!files.is_empty(), "必须真的读到源码");
    let mut missing: Vec<String> = Vec::new();
    for (variant, modules) in EXPECTED_SITES {
        let mut found_any = false;
        for module in modules {
            let Some(text) = module_text(&files, module) else {
                missing.push(format!("{variant}: 读不到模块 {module}"));
                continue;
            };
            if constructed_variants(&text).contains(variant) {
                found_any = true;
            } else {
                missing.push(format!("{variant}: {module} 里没有构造点"));
            }
        }
        if !found_any {
            missing.push(format!("{variant}: 一个构造点都没有"));
        }
    }
    assert!(
        missing.is_empty(),
        "这些 (变体, 模块) 的构造点不见了（工具面因此可能不可达）: {missing:#?}"
    );
    // 登记表必须覆盖**恰好** N 个变体（N = 本表长度），而且名字不重复。
    let names: BTreeSet<&str> = EXPECTED_SITES.iter().map(|(name, _)| *name).collect();
    assert_eq!(names.len(), EXPECTED_SITES.len(), "变体名不得重复");
    assert_eq!(
        names.len(),
        49,
        "登记表有 49 条（⭐ 它**不是**模型变体数 —— 模型全集由 \
         `every_model_op_variant_is_registered_or_declared_unreachable` 管）"
    );
}

/// ⭐ **正对照（第 42 条）＋ 防空**：抽取器必须**真的**抽到东西，且**至少**含一条
/// **已知存在**的 kind。
///
/// ## 为什么必须有它（与 `the_catalog_kinds_are_exactly_the_parse_arms` 互补）
///
/// 那条判据是**双向差集** ⇒ 抽取器抽到**空集**时它**仍会红**（⛔ 不假绿）。⚠ 但它会红成
/// 「只在目录里（没有解析臂）」—— ⭐ **把"抽取器瞎了"报成"目录写错了"**。
/// 本判据把那个区分**显式**做出来：先证明"抽取器是活的"（非空 ＋ 含已知样例），
/// 再让对账那条去判"两边是否相等"。
///
/// ⚠ 样例**从 [`OP_KINDS`] 现取**（⛔ 不写死一个可能被改名/删除的字面）：它是权威来源，
/// 而"权威里的一条必须能被抽出"正是本条要钉的。
///
/// ⚠ 仓内范式同形：`project.rs` 的 `routing_graph_fields_in_source` 用 `.expect("结构体定义")`
/// 让锚点缺失**响亮失败**；`determinism_digests.rs` 的 `identical_sides` 对已知样例做断言。
///
/// 注入（实测红）：把 `find("fn parse_one(")` 改成 `find("fn parse_one_XX(")` ⇒ 本条
/// （与对账那条）**都**红，且报文**指名抽取规则失效**、⛔ 不是"只在目录里"。
#[test]
fn the_parse_arm_extractor_is_alive() {
    let files = sources();
    let notes = module_text(&files, "domain/notes.rs").expect("读得到 notes.rs");
    let arms = kinds_in_parse_one(&notes);

    assert!(
        !arms.is_empty(),
        "⭐ 抽取规则失效（⛔ 不是目录的问题）: `kinds_in_parse_one` 抽出了 **0** 条 —— \
         锚点或函数体形状变了。⛔ 不要据此去改 `OP_KINDS`"
    );
    // ⭐ 正对照：权威里**每一条**都必须能被抽出（这不是对账，是"抽取器活着"的证明）。
    let missing: Vec<&str> = OP_KINDS
        .iter()
        .copied()
        .filter(|kind| !arms.contains(*kind))
        .collect();
    assert!(
        missing.is_empty(),
        "⭐ 抽取器漏掉了 `OP_KINDS` 里的这些 kind: {missing:?} —— \
         先怀疑**抽取规则**（锚点/切片/换行），⛔ 不要先怀疑目录"
    );
}

#[test]
fn the_catalog_kinds_are_exactly_the_parse_arms() {
    let files = sources();
    let notes = module_text(&files, "domain/notes.rs").expect("读得到 notes.rs");
    let arms = kinds_in_parse_one(&notes);
    let catalog: BTreeSet<String> = OP_KINDS.iter().map(|kind| (*kind).to_owned()).collect();
    let only_catalog: Vec<&String> = catalog.difference(&arms).collect();
    let only_arms: Vec<&String> = arms.difference(&catalog).collect();
    assert!(
        only_catalog.is_empty() && only_arms.is_empty(),
        "`OP_KINDS` 与 `parse_one` 的 match 臂**逐字**必须互为双射。\n\
         只在目录里（没有解析臂）: {only_catalog:?}\n只在解析臂里（没登记）: {only_arms:?}"
    );
}

/// 分类器**真的**分得开"构造"与"模式"（本判据喂纯合成的三行文本）。
///
/// 为什么必须有这一条：`every_op_variant_has_a_registered_construction_site` 的
/// 结论完全依赖 [`constructed_variants`]，而它的三条规则是从真实源码里"看出来"的。
/// 没有这一条时，把规则②（紧跟 `=>`）删掉，真实源码上**看不出差别**
/// （因为那些模式旁边还有别的判据兜着）—— 那是"冗余防线"的经典形状。
///
/// 注入（实测红）：删掉规则②（`if text[after..].starts_with("=>")`）、
/// 规则①（`matches!(`）或规则③（`|`）中的任一条 ⇒ 本判据红。
#[test]
fn the_classifier_tells_constructions_from_patterns() {
    // 构造：`push(...)`、`let x = ...`、`=> Op::V {`（右值是构造）。
    let constructions = concat!(
        "ops.push(Op::AddTrack { track: t });\n",
        "let op = Op::SetAutomationPoint { lane, tick, value, curve };\n",
        "Self::Mute => Op::SetTrackMute { track_id, old_mute, new_mute },\n",
        "Ok(Op::AddNote { track_id, clip_id, note })\n",
    );
    assert_eq!(
        constructed_variants(constructions),
        ["AddNote", "AddTrack", "SetAutomationPoint", "SetTrackMute"]
            .into_iter()
            .map(str::to_owned)
            .collect()
    );
    // 模式：match 臂 / `matches!` / 或模式交替 —— 一个都不许被当成构造。
    let patterns = concat!(
        "match op { Op::AddClip { clip } => { let _ = clip; } _ => {} }\n",
        "if matches!(op, Op::AddClip { .. }) { return; }\n",
        "matches!(op, Op::AddNote { .. } | Op::DeleteNote { .. });\n",
    );
    // ⭐ R94：**注释与字符串里的 `Op::X {{` 不是构造点**（本函数此前完全没有掩码）。
    let quoted = concat!(
        "// 文档里写 Op::SetSection {{ section_id, .. }} 只是引用\n",
        "let sample = \"Op::AddTrack {{ track: t }}\";\n",
        "let real = Op::RemoveScene {{ scene_id }}; // 尾注释里写 Op::SetScene {{ .. }}\n",
    );
    let found = constructed_variants(quoted);
    assert_eq!(
        found,
        ["RemoveScene"].into_iter().map(str::to_owned).collect(),
        "只有真构造点该被读到（注释/字符串里的引文必须跳过）"
    );
    assert!(
        constructed_variants(patterns).is_empty(),
        "模式不得被当成构造: {:?}",
        constructed_variants(patterns)
    );
    // `if let A | B = op` 形态：`matches!` 与 `=>` 都不在场 ⇒ 只有规则③（B 前面是 `|`）
    // 与规则④（A 后面是 `|`）能判它。
    assert!(
        constructed_variants("if let Op::AddNote { .. } | Op::DeleteNote { .. } = op {}")
            .is_empty(),
        "`if let A | B` 的两支都不许被当成构造"
    );
    // `NoteOp::` 前缀不算（本 crate 的两个枚举前缀重叠，这是最容易错的字面陷阱）。
    assert!(
        constructed_variants("let x = NoteOp::AddNote { track_id, clip_id, note };").is_empty()
    );
    // 单元变体（后面不跟 `{`）算构造。
    assert!(constructed_variants("let x = Op::RemoveClip;").contains("RemoveClip"));
}

/// ⭐ **反向断言**：模型 `Op` 的**每一个**变体都必须在 `EXPECTED_SITES` 里登记，
/// 或者出现在 `KNOWN_UNREACHABLE` 的显式欠账清单里。
///
/// ⭐ 为什么必须有它：`every_op_variant_has_a_registered_construction_site` 只对
/// `EXPECTED_SITES` **里已有的项**遍历（"已登记项有构造点"）⇒ ⭐ **它是单向的**：
/// 一个模型变体若**根本没被登记**，那条判据**看不见它**（实测：模型 33 变体而登记表
/// 31 条，该判据仍然全绿 ⇒ 绿着漏掉 2 个）。
///
/// ⚠ 本判据**从模型源码现读**变体名（⛔ 不手抄数字）⇒ 模型加一个变体不登记就会红。
/// ⚠ 抽取规则的两条教训（见台账 R417）：⛔ **不要用花括号配对**（模型的文档注释里
/// 充满 `Op::X { .. }` 示例 ⇒ 会提前结束、静默读到 0 个）；并且"抽不到东西"必须
/// **响亮失败**，否则抽取器失效时本判据会**永远绿**（又一个空绿）。
#[test]
fn every_model_op_variant_is_registered_or_declared_unreachable() {
    let crates_dir = manifest_dir()
        .parent()
        .expect("crates/ 必须存在")
        .to_path_buf();
    let model_files = read_rust_sources(&[crates_dir.join("yeban-model/src")]);
    let ops = module_text(&model_files, "/ops.rs").expect("读得到 yeban-model 的 ops.rs");

    // 从 `pub enum Op {` 起、扫到**行首 `}`** 止（⛔ 不配对：注释里有 `{}` 示例）。
    let start = ops.find("pub enum Op {").expect("找得到 `pub enum Op {`");
    let mut body = String::new();
    for line in ops[start..].lines().skip(1) {
        if line == "}" {
            break;
        }
        body.push_str(line);
        body.push('\n');
    }

    let mut model: BTreeSet<String> = BTreeSet::new();
    for line in body.lines() {
        let Some(rest) = line.strip_prefix("    ") else {
            continue;
        };
        if rest.starts_with(' ') {
            continue; // 缩进更深 ⇒ 是字段，不是顶层变体
        }
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if name.is_empty() {
            continue;
        }
        // ⚠ 变体行形如 `AddNote {` / `RemoveTrack {` ⇒ 名字之后是**空格 ＋ `{`**；
        // 必须先 `trim_start()`（实测踩到：直接判 `starts_with('{')` 会永远不匹配 ⇒ 读到 0 个）。
        let tail = rest[name.len()..].trim_start();
        if tail.starts_with('{') || tail.starts_with(',') {
            model.insert(name);
        }
    }
    assert!(
        model.len() > 20,
        "从模型源码读出的变体数太少（{}）—— 抽取规则失效了，本判据**不可信**（⛔ 不许当绿）",
        model.len()
    );

    let registered: BTreeSet<String> = EXPECTED_SITES
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();
    let declared: BTreeSet<String> = KNOWN_UNREACHABLE
        .iter()
        .map(|(n, _)| (*n).to_owned())
        .collect();

    let unregistered: Vec<&String> = model
        .difference(&registered)
        .filter(|name| !declared.contains(*name))
        .collect();
    assert!(
        unregistered.is_empty(),
        "这些模型 `Op` 变体既没进 `EXPECTED_SITES`、也没进 `KNOWN_UNREACHABLE` \
         ⇒ ⭐ 加变体的人必须二选一（登记构造点，或如实登记为不可达）: {unregistered:?}"
    );

    // 反向的另一半：欠账清单不许留**已登记**或**模型里已不存在**的陈旧条目。
    for (name, reason) in KNOWN_UNREACHABLE {
        assert!(
            model.contains(name),
            "`KNOWN_UNREACHABLE` 里的 `{name}` 在模型里已不存在 ⇒ 请删掉本条（原文理由: {reason}）"
        );
        assert!(
            !registered.contains(name),
            "`{name}` 既在 `KNOWN_UNREACHABLE` 又在 `EXPECTED_SITES` ⇒ 两处矛盾，请二选一"
        );
    }
}

/// ⭐ **对账守卫（第 44 条）**：`yeban_edit_notes` 的**实参说明**必须**逐个提到**
/// `OP_KINDS` 里的每个 `kind`。
///
/// ⭐ 为什么需要它：那份说明是**手抄的清单**，⛔ 没有任何东西与 `OP_KINDS` 对账 ⇒
/// 它会**静默漂移**。实证：`setTransportConfig` 已实现（`OP_KINDS`、解析臂、
/// compile 臂都在），而说明里**一个字都没有** —— 那一轮描述改动**在变基中丢失**，
/// **整整一轮无人发现**（⛔ 不是"跑一遍判据就能看出来"的那种）。
///
/// ⚠ 与 `every_model_op_variant_is_registered_or_declared_unreachable` **同法**：
/// 清单必须覆盖**权威来源**（这里权威是 `OP_KINDS` 本身，⛔ 不是再手抄一份）。
#[test]
fn the_argument_help_text_mentions_every_kind() {
    let files = sources();
    let tools = module_text(&files, "/tools.rs").expect("读得到 tools.rs");

    // ⚠ **不许用 `tools.contains(kind)`（裸子串）**：`add` 是 `addTrack`／`addRoutingNode`
    // 的**子串** ⇒ 那种检查会被**蒙过**（实测：`add` 是全文唯一不以 `` `X` `` 形式列举的
    // kind，裸子串检查却报绿）。只认**带定界的两种形态**：`` `kind` `` 或 `"kind"`。
    // ⚠ 带引号的那半边能安全排除子串误命中：`"add"` 的闭引号不会出现在 `"addTrack"` 里。
    let missing: Vec<&str> = OP_KINDS
        .iter()
        .copied()
        .filter(|kind| {
            let backticked = format!("`{kind}`");
            let quoted = format!("\"{kind}\"");
            !tools.contains(&backticked) && !tools.contains(&quoted)
        })
        .collect();

    assert!(
        missing.is_empty(),
        "`yeban_edit_notes` 的实参说明漏列这些 kind（⭐ 说明是手抄清单，\
         加/删 kind 时必须同步它）: {missing:?}"
    );
}

/// ⭐ **反向那一半**（R432 登记、本单补）：枚举里的每个 kind 记号都**必须**在
/// `OP_KINDS` 里。
///
/// ## 为什么需要它（正向那半看不见什么）
///
/// [`the_argument_help_text_mentions_every_kind`] 只查 `OP_KINDS ⊆ 说明`。⚠ 若说明里
/// **留下一个已删除的 kind**（或写错一个名字），它**看不见** —— 加了 kind 会红，
/// **删了/写错不会**。本判据补的正是那半边。
///
/// ## 抽取规则（机械、可复核 ⛔ 不是黑箱）
///
/// ⚠ 难点：那段说明里有 **~480** 个反引号词（字段名、`Op` 名、`reason` 名、路径…），
/// ⛔ 不能"所有反引号词都当 kind"。
///
/// ⭐ 正解**不是**靠"看起来像 kind"的**形状**规则：实测那条形状规则（小写开头 ＋
/// 驼峰）**召回只有 22/26** —— 纯小写的 `add` / `move` / `delete` / `velocity`
/// 全被它筛掉，而它在整个 `tools.rs` 上有 **98** 个假阳性 ⇒ 两头都不可靠。
///
/// ⭐ 本判据改用**位置**这一条机械规则：`yeban_edit_notes` 的说明里，
/// **枚举段**写成 `` / `X` / `Y` / … ``。因此只取**该形状**里的词，取法是从**第一个**
/// `` / `X` `` 起，连续吃 `` / `X` `` 直到形状断开为止。⛔ 不按"像不像 kind"筛。
///
/// 这条规则**自己也是可复核的**：判据把"吃了几个、断在哪"作为读数报出来，并且
/// 另有一条判据 [`every_enumerated_kind_matches_the_shape_rule`] 核对"枚举里的词确实
/// 都满足形状规则"，于是形状规则**不承担过滤职责**、只作为交叉验证。
///
/// ## 防空（第 41 条）
///
/// 枚举抽出 **0** 个词 ⇒ 断言（一个失灵的起点查找会让"全部都在 `OP_KINDS` 里"
/// 永远成立）。
///
/// 注入（实测红）：在枚举里加一个 `` `zzGhostKind` `` ⇒ 本判据点名它。
#[test]
fn every_enumerated_kind_is_in_the_catalog() {
    let files = sources();
    let tools = module_text(&files, "/tools.rs").expect("读得到 tools.rs");

    let (enumerated, stopped_at) = enumeration_tokens(&tools);
    assert!(
        !enumerated.is_empty(),
        "枚举段抽出了 0 个词 —— 起点 `` / `X` `` 的查找已失灵, ⛔ 不许静默通过"
    );
    assert!(
        enumerated.len() >= 20,
        "枚举段只抽出 {} 个词 ⇒ 抽取规则已失灵（现读起点断在 {stopped_at:?}）",
        enumerated.len()
    );

    let catalog: BTreeSet<&str> = OP_KINDS.iter().copied().collect();
    let unknown: Vec<&str> = enumerated
        .iter()
        .copied()
        .filter(|token| !catalog.contains(token))
        .collect();
    assert!(
        unknown.is_empty(),
        "`yeban_edit_notes` 的实参说明枚举里出现了**不在 `OP_KINDS` 里**的 kind: {unknown:?} \
         ⇒ 要么把它加进 `OP_KINDS`（并实现它），要么从说明里删掉 \
         (⛔ 不许把本判据放宽成单向)"
    );

    // 逐条读数：枚举词数 ＋ 抽取断在哪（⛔ 不让抽取器变成黑箱）。
    assert!(
        !stopped_at.is_empty(),
        "抽取必须停在某个可读的记号上（现读: {stopped_at:?}）"
    );
}

/// **交叉验证形状规则**：枚举段里的词**全部**满足"像 kind"的形状（小写开头 ＋ 驼峰
/// ＋ 纯 ASCII 字母）。
///
/// ⚠ 这条**不**用于过滤（过滤靠 [`enumeration_tokens`] 的位置规则）—— 它存在的
/// 意义是把"形状规则到底筛掉几条"变成**显式读数**：若枚举里出现一个**不**满足形状
/// 的词，说明位置规则吃进了非 kind 的东西（⛔ 那是抽取器变宽，必须看见）。
///
/// ⚠ 例外是**明文登记**的：纯小写的 kind（`add` / `move` / `delete` / `velocity`）
/// 天然**不**满足驼峰形状 ⇒ 它们进 [`SHAPE_RULE_EXEMPT_KINDS`]，逐条带理由。
#[test]
fn every_enumerated_kind_matches_the_shape_rule() {
    let files = sources();
    let tools = module_text(&files, "/tools.rs").expect("读得到 tools.rs");
    let (enumerated, _) = enumeration_tokens(&tools);

    let exempt: BTreeSet<&str> = SHAPE_RULE_EXEMPT_KINDS.iter().copied().collect();
    let violations: Vec<&str> = enumerated
        .iter()
        .copied()
        .filter(|token| !exempt.contains(token) && !looks_like_kind(token))
        .collect();
    assert!(
        violations.is_empty(),
        "枚举段里这些词⛔ 不满足形状规则、也不在豁免清单里 ⇒ 位置规则吃进了非 kind: \
         {violations:?}"
    );
}

/// ⚠ **明文豁免**：合法 kind 里**天然不满足**"小写开头 ＋ 驼峰"形状的那些（纯小写词）。
///
/// 与 `EXEMPT_SCHEMA_KEYS` 同型：每条带理由。⭐ 它同时也是"形状规则召回不满"的
/// **逐字证据**（22/26），因此**不**拿形状规则当过滤器。
const SHAPE_RULE_EXEMPT_KINDS: [&str; 4] = ["add", "delete", "move", "velocity"];

/// 词是否是"像 kind"的形状：非空 ＋ 纯 ASCII 字母 ＋ 小写开头 ＋ 至少一个大写。
///
/// ⚠ 实测：这条规则在 `OP_KINDS` 上**召回 22/26**（漏掉纯小写的四个），在整个
/// `tools.rs` 上**假阳性 98** ⇒ ⛔ 它**不能**单独当抽取器。本文件只把它当**交叉
/// 验证**（见 [`every_enumerated_kind_matches_the_shape_rule`]）。
fn looks_like_kind(token: &str) -> bool {
    !token.is_empty()
        && token.bytes().all(|byte| byte.is_ascii_alphabetic())
        && token.starts_with(|first: char| first.is_ascii_lowercase())
        && token.bytes().any(|byte| byte.is_ascii_uppercase())
}

/// 从 `tools.rs` 里抽 `yeban_edit_notes` 实参说明的**枚举段**。
///
/// 规则（机械、⛔ 不按"像不像 kind"筛）：先找 `操作列表 (NoteOp)`，再从那里找**第一个**
/// `` / `X` `` 形状；此后**连续**吃同样的形状，直到形状断开为止。
///
/// 返回（抽到的词, 断开处的片段）—— 断开的片段是**读数**，⛔ 不让抽取器变黑箱。
fn enumeration_tokens(tools: &str) -> (Vec<&str>, String) {
    const HELP_HEADING: &str = "操作列表 (NoteOp)";
    let Some(from) = tools.find(HELP_HEADING) else {
        return (Vec::new(), String::new());
    };
    let rest = &tools[from..];
    let Some((first_start, _)) = first_enumerated_token(rest) else {
        return (Vec::new(), String::new());
    };

    let mut tokens = Vec::new();
    let mut cursor = first_start;
    while let Some((offset, token)) = first_enumerated_token(&rest[cursor..]) {
        // 枚举项之间只允许空白 —— 出现别的字符说明枚举段已经结束。
        let gap = &rest[cursor..cursor + offset];
        if !gap.chars().all(char::is_whitespace) {
            break;
        }
        tokens.push(token);
        // 推进到**该项的闭反引号之后**：
        // `/` ＋ 空白 ＋ `` ` `` ＋ token ＋ `` ` ``。
        let after_open = cursor + offset + 1;
        let ws = rest[after_open..]
            .chars()
            .take_while(|c| c.is_whitespace())
            .count();
        cursor = after_open + ws + 1 + token.len() + 1;
    }
    let stopped_at: String = rest[cursor..].chars().take(40).collect();
    (tokens, stopped_at)
}

/// 在 `text` 里找**第一个** `` / `X` `` 形状，返回（相对偏移, 词）。
fn first_enumerated_token(text: &str) -> Option<(usize, &str)> {
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'/' {
            let mut probe = index + 1;
            while probe < bytes.len() && (bytes[probe] == b' ' || bytes[probe] == b'\t') {
                probe += 1;
            }
            if probe < bytes.len() && bytes[probe] == b'`' {
                let token_start = probe + 1;
                let mut end = token_start;
                while end < bytes.len() && bytes[end].is_ascii_alphabetic() {
                    end += 1;
                }
                if end > token_start && end < bytes.len() && bytes[end] == b'`' {
                    return Some((index, &text[token_start..end]));
                }
            }
        }
        index += 1;
    }
    None
}
