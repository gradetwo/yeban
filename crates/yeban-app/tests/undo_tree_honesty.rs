//! **诚实性审计（文本层 + 写者层）** —— 两处"界面声称有数据、却没有宿主写者"的位置。
//!
//! 本文件是 `tests/live_ui_mcp.rs` 的运行时判据的**另一半**：本机不渲染 Slint 时也能跑，
//! 且多条路径（源码文本 / 属性默认值 / 写者计数 / 结构）各自独立，破坏任何一条都变红。
//!
//! | 判据 | 钉住什么 |
//! | :--- | :--- |
//! | [`the_undo_tree_declares_no_demo_history_nodes`] | 六个编造的版本名（含 `"c5 AI 提案"`）不得回到界面的任何 `.slint` 里；节点数据面默认必须是空的；节点循环必须由**数据**驱动 |
//! | [`the_undo_tree_node_properties_have_exactly_one_writer`] | 三个节点属性各自**恰好一个**写者，且那个写者读的是**权威图谱**（`UndoPort::graph()`），不是第二份状态 |
//! | [`the_ai_badge_claims_no_pending_proposal_and_has_a_real_action`] | 徽章不得再声称「待审查 / 1 条」，且 `accessible-role: button` 必须有**真的**点击源与回调 |
//!
//! 为什么这批判据住在 `tests/` 而不是 `src/elements.rs` 的单元测试里：本票与另一条
//! 工作线共用同一个 worktree，`src/elements.rs` 的 `mod tests` 尾部由那条线在改
//! （见本票报告）。判据放在新文件里 ⇒ 两个写者的改动面不重叠。

use std::path::{Path, PathBuf};

use yeban_app::elements::DEMO_UNDO_TREE_LITERALS;

/// 从 `ui/transport.slint` 的 AI 徽章上**删掉的**假状态字面量（逐字）。
///
/// 改动前 `transport-ai-proposal-badge` 的 `accessible-label` 是
/// `"AI 编曲提案待审查"`、可见文案是 `"AI提案 (待审查)"`、`accessible-value` 是 `"1"`，
/// 而**没有任何宿主写者**（`git grep -n 'ai-proposal-badge'` 只命中语义注册表与测试）
/// ⇒ 用户看到的是「有 1 条 AI 提案待审查」这个不存在的状态。
///
/// 「待审查」这三个字是那个假状态**唯一**的承载词，所以它是探针的字面量：
/// `ui/**` 的源码文本、`src/**` 的注册表标签、以及运行时树的
/// `accessible-label` / `accessible-value` 三侧都不得命中它。
const BADGE_FALSE_LITERALS: [&str; 1] = ["待审查"];

/// `crates/yeban-app/`。
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn ui_dir() -> PathBuf {
    crate_dir().join("ui")
}

fn read(rel: &str) -> String {
    let path = crate_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()))
}

/// 去掉 `//` 行注释与行尾注释后的**代码文本**。
///
/// 为什么要去注释：本票把「什么被删掉了」写进注释（审计留痕）。探针若把注释也算成
/// 「界面里的字面量」，就会逼着作者**不写**留痕 —— 那与本仓库的纪律相反。
fn code_only(source: &str) -> String {
    source
        .lines()
        .map(|line| match line.split_once("//") {
            Some((code, _comment)) => code,
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `ui/**/*.slint` 的全部文件（相对 `crates/yeban-app/` 的路径 + 源码）。
fn all_slint() -> Vec<(String, String)> {
    fn walk(dir: &Path, out: &mut Vec<(String, String)>) {
        let entries = std::fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("读目录 {} 失败: {error}", dir.display()));
        for entry in entries {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "slint") {
                let text = std::fs::read_to_string(&path)
                    .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()));
                let rel = path
                    .strip_prefix(crate_dir())
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push((rel, text));
            }
        }
    }
    let mut out = Vec::new();
    walk(&ui_dir(), &mut out);
    out.sort();
    assert!(!out.is_empty(), "`ui/` 里一个 .slint 都没有");
    out
}

/// `src/**/*.rs` 的全部**代码**文本（去掉 `//` 注释后拼接）。
///
/// 为什么要去注释：本票把「什么被删掉了」写进注释（审计留痕），注释里当然会出现
/// 被删掉的原文。探针若把注释也算成「界面里的字面量」，就会逼着作者**不写**留痕 ——
/// 那与本仓库的纪律相反（与 `code_only` 对 `.slint` 的口径同款）。
fn all_src_rs() -> String {
    fn walk(dir: &Path, out: &mut String) {
        for entry in std::fs::read_dir(dir).expect("读 src 目录") {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push_str(&code_only(&std::fs::read_to_string(&path).unwrap_or_else(
                    |error| panic!("读 {} 失败: {error}", path.display()),
                )));
                out.push('\n');
            }
        }
    }
    let mut out = String::new();
    walk(&crate_dir().join("src"), &mut out);
    assert!(out.len() > 10_000, "`src/` 的源码拼接异常偏小");
    out
}

/// `needle` 在 `haystack` 里出现的**次数**（单位：次）。
fn count(haystack: &str, needle: &str) -> usize {
    haystack.matches(needle).count()
}

/// `source` 里以 `{name}:` **开头**的行数（去掉行首空白后比较）。
///
/// 为什么不用 `count(source, "name:")`：`undo-node-labels:` 里就含有
/// `node-labels:` 这个子串，子串计数会把两个**不同**的属性算成同一个。
fn assignment_lines(source: &str, name: &str) -> Vec<String> {
    source
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with(&format!("{name}:")))
        .map(str::to_owned)
        .collect()
}

// =========================================================================
// ① 时光机：编造的六个版本节点不许回来
// =========================================================================

/// 判据（**多路径探针：全 `ui/` 源码文本 + 属性默认值 + 循环规模 + 空态图元**）。
///
/// 四条路径各自独立：
///
/// 1. **全 `ui/` 源码文本**：[`DEMO_UNDO_TREE_LITERALS`] 一条都不得出现在任何 `.slint`
///    的**代码**里。只查 `undo_tree_modal.slint` 不够 —— 把假节点搬到 `app.slint`
///    再注入，用户看到的还是一模一样的假历史。
/// 2. **属性默认值**：三个数据面属性的默认值必须是 `0` / `[]` / `0`。这一条钉的是
///    「宿主不写时界面显示什么」。
/// 3. **循环规模**：节点列表必须由**数据**驱动（`for … in root.node-labels`），
///    不得是固定的 `for … in 6` —— 固定循环加空数组会在界面上留六张空卡片。
/// 4. **空态 / 截断报数**：两个状态都必须有稳定的语义 ID（用户可见，不是注释）。
#[test]
fn the_undo_tree_declares_no_demo_history_nodes() {
    // ---- 路径 1：全 ui/ 的代码文本
    let mut leaks: Vec<String> = Vec::new();
    for (rel, text) in all_slint() {
        let code = code_only(&text);
        for literal in DEMO_UNDO_TREE_LITERALS {
            if code.contains(literal) {
                leaks.push(format!("{rel}: 代码里出现 `{literal}`"));
            }
        }
    }
    assert!(
        leaks.is_empty(),
        "时光机里出现了编造的版本节点名（用户会把它当成真实历史）:\n  {}",
        leaks.join("\n  ")
    );

    // ---- 路径 2：属性默认值
    let modal = code_only(&read("ui/dialogs/undo_tree_modal.slint"));
    for required in [
        "in property <int> node-count: 0;",
        "in property <[string]> node-labels: [];",
        "in property <int> node-hidden: 0;",
    ] {
        assert!(
            modal.contains(required),
            "`ui/dialogs/undo_tree_modal.slint` 必须如实声明 `{required}` —— \
             没有宿主写者时界面必须画空态，而不是六个编造的版本"
        );
    }

    // ---- 路径 3：循环规模由数据决定
    assert!(
        modal.contains("for node_label[node_index] in root.node-labels"),
        "节点列表必须由**数据**驱动（`for node_label[node_index] in root.node-labels`）"
    );
    assert!(
        !modal.contains("for node_index in 6"),
        "节点列表不得是固定的 `for node_index in 6`（规模必须来自真实提交链）"
    );
    assert!(
        modal.contains("accessible-item-count: root.node-count;"),
        "每个节点的 `accessible-item-count` 必须等于真实节点数（写死 6 就是假读数）"
    );

    // ---- 路径 4：空态与截断报数必须是**用户可见**的图元
    assert!(
        modal.contains("accessible-id: \"undo-tree-empty-state\""),
        "链为空时必须有用户可见的空态节点 `undo-tree-empty-state`"
    );
    assert!(
        modal.contains("accessible-id: \"undo-tree-truncation-note\""),
        "链条比画布长时必须**如实报数**（`undo-tree-truncation-note`），不许静默截断"
    );
}

// =========================================================================
// ② 时光机：三个节点属性各自恰好一个写者，且那个写者读权威图谱
// =========================================================================

/// 判据（**写者计数 + 唯一性 + 权威路径**）。
///
/// 四个方向：
///
/// 1. `src/**` 里每个 `set_undo_node_*(` 都**恰好出现一次** —— 两个写者就是两个真相源；
/// 2. `MainWindow` 必须声明这三个属性（否则 setter 无从谈起）；
/// 3. `ui/**` 里 `node-labels:` / `node-count:` / `node-hidden:` 的**赋值行**各恰好两处：
///    弹窗的默认值 + `app.slint` 的转发（`.slint` 侧不得自带第三份数据）；
/// 4. 那个唯一的写者必须走**权威读路径**：`host.rs` 的 `apply_undo` 里出现
///    `undo_tree_projection(port)`，而该函数读 `port.graph()` + `graph.ancestry(` ——
///    即数据来自 `CommitGraph`，不是宿主另造的一份列表。
#[test]
fn the_undo_tree_node_properties_have_exactly_one_writer() {
    let src = all_src_rs();
    for prop in ["undo-node-count", "undo-node-labels", "undo-node-hidden"] {
        let setter = format!("set_{}(", prop.replace('-', "_"));
        assert_eq!(
            count(&src, &setter),
            1,
            "`{setter}` 在 `src/**` 里必须恰好出现一次（唯一写者），实际 {} 次",
            count(&src, &setter)
        );
    }

    let app = code_only(&read("ui/app.slint"));
    for (prop, ty) in [
        ("undo-node-count", "int"),
        ("undo-node-labels", "[string]"),
        ("undo-node-hidden", "int"),
    ] {
        assert!(
            app.contains(&format!("in-out property <{ty}> {prop}:")),
            "`MainWindow` 必须声明 `in-out property <{ty}> {prop}: …`（宿主 setter 的唯一落点）"
        );
    }

    // 转给弹窗：`app.slint` 里各恰好一条赋值行，且值来自 MainWindow 的属性。
    assert_eq!(
        assignment_lines(&app, "node-count").len(),
        1,
        "`app.slint` 里 `node-count:` 必须恰好一条赋值行"
    );
    assert_eq!(
        assignment_lines(&app, "node-labels").len(),
        1,
        "`app.slint` 里 `node-labels:` 必须恰好一条赋值行"
    );
    assert_eq!(
        assignment_lines(&app, "node-hidden").len(),
        1,
        "`app.slint` 里 `node-hidden:` 必须恰好一条赋值行"
    );
    assert!(
        app.contains("node-labels: root.undo-node-labels;"),
        "`app.slint` 必须把 `root.undo-node-labels` 转给时光机（界面不自带数据）"
    );
    assert!(
        app.contains("node-count: root.undo-node-count;")
            && app.contains("node-hidden: root.undo-node-hidden;"),
        "`app.slint` 必须把节点数与截断数一并转给时光机"
    );

    // 权威路径：唯一写者读的是 `UndoPort::graph()`（`CommitGraph`），不是自造列表。
    let host = read("src/host.rs");
    let apply_undo = host
        .split("pub fn apply_undo")
        .nth(1)
        .and_then(|rest| rest.split("\n}\n").next())
        .expect("`host.rs` 里必须有 `apply_undo`");
    assert!(
        apply_undo.contains("undo_tree_projection(port)"),
        "`apply_undo` 必须经 `undo_tree_projection` 取真实节点"
    );
    assert!(
        host.contains("let graph = port.graph();") && host.contains("graph.ancestry(&head)"),
        "节点数据必须来自**权威图谱**（`UndoPort::graph()` + `CommitGraph::ancestry`）—— \
         自己攒一份列表就是第二个权威"
    );
    assert!(
        host.contains("chain.len().saturating_sub(UNDO_TREE_MAX_NODES)"),
        "被画布截掉的版本数必须由链条长度算出（不是估计值）"
    );
}

// =========================================================================
// ③ AI 徽章：不许再声称「有待审查的提案」，且按钮角色必须有真动作
// =========================================================================

/// 判据（**多路径探针：`ui/` 源码文本 + `src/` 注册表标签 + 角色/动作结构 + 转发**）。
///
/// 改动前这一处是**三连假**：`accessible-role: button` 没有点击源；
/// `accessible-value: "1"` 与可见文案「AI提案 (待审查)」声称有 1 条待审查提案，
/// 而没有任何宿主写者。四条路径各自独立：
///
/// 1. 假状态字面量（[`BADGE_FALSE_LITERALS`]）不得出现在 `ui/**` 的代码里；
/// 2. 它也不得出现在 `src/**`（语义注册表的标签会进控制树产物，用户与自动化都读得到）；
/// 3. 徽章块里不得有 `accessible-value`（任何写死的条数都是编造的读数）；
/// 4. `accessible-role: button` 必须配一个真的 `TouchArea` + 回调，且 `app.slint`
///    把它转到**已存在**的那个宿主动作（`open-musical-pr`）。
#[test]
fn the_ai_badge_claims_no_pending_proposal_and_has_a_real_action() {
    let transport = code_only(&read("ui/transport.slint"));

    // ---- 路径 1：ui/ 代码文本
    for (rel, text) in all_slint() {
        let code = code_only(&text);
        for literal in BADGE_FALSE_LITERALS {
            assert!(
                !code.contains(literal),
                "{rel}: 出现了假状态字面量 `{literal}` —— 提案条数今天没有数据源，\
                 任何「有几条待审查」的说法都是编造的"
            );
        }
    }

    // ---- 路径 2：注册表标签（`src/**`）
    let src = all_src_rs();
    for literal in BADGE_FALSE_LITERALS {
        assert!(
            !src.contains(literal),
            "`src/**` 里还有假状态字面量 `{literal}`（语义注册表会把它写进控制树产物）"
        );
    }

    // ---- 路径 3 + 4：徽章块的结构
    let badge_start = transport
        .find("accessible-id: \"transport-ai-proposal-badge\";")
        .expect("`transport-ai-proposal-badge` 必须留在 `transport.slint` 里");
    let badge_end = transport[badge_start..]
        .find("\n    }\n")
        .map(|offset| badge_start + offset)
        .expect("徽章块必须有闭合");
    let badge = &transport[badge_start..badge_end];
    assert!(
        !badge.contains("accessible-value"),
        "徽章块里不得有 `accessible-value`：提案条数今天没有数据源，写死就是编造读数。\n{badge}"
    );
    assert!(
        badge.contains("text: \"AI提案\""),
        "徽章的可见文案必须是 `\"AI提案\"`（不得再带「待审查」这类状态声明）"
    );
    assert!(
        badge.contains("clicked => { root.open-ai-proposals(); }"),
        "`accessible-role: button` 必须配一个真的 `TouchArea`（否则是「点了没反应」的假控件）。\n{badge}"
    );
    assert!(
        transport.contains("callback open-ai-proposals();"),
        "`transport.slint` 必须声明 `callback open-ai-proposals();`"
    );

    // 转发到**已存在**的宿主动作（不另造落点）。
    let app = code_only(&read("ui/app.slint"));
    assert!(
        app.contains("open-ai-proposals => {") && app.contains("root.open-musical-pr();"),
        "`app.slint` 必须把 `open-ai-proposals` 转到既有的 `open-musical-pr` \
         （`musical-pr-open` 的唯一写者仍是宿主）"
    );
    assert!(
        count(&app, "root.open-musical-pr();") >= 2,
        "徽章与右栏按钮必须落到**同一个** `open-musical-pr` 回调"
    );
}
