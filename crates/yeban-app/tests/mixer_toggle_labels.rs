//! **撤销树节点标签的诚实性** —— 台账缺口 **R17**。
//!
//! ## 缺陷（本文件钉住的就是它）
//!
//! `crates/yeban-app/src/host.rs` 的 `wire_mixer_switch` 是静音 / 独奏两个开关的**唯一**
//! 接线实现，而那条 `commit_ops` 的消息就是撤销树的**节点标签**（`UndoPort::commit_ops`
//! → `Commit::message`）。它原先写死 `"mixer: toggle track mute"` / `"mixer: toggle track solo"`，
//! 但本函数有**两个**界面入口：
//!
//! | 入口 | 文件 | 到宿主的路 |
//! | :--- | :--- | :--- |
//! | 调音台通道条 | `ui/console/mixer_console.slint` | `root.mixer-mute-toggle(track_index)` → `MainWindow.mixer-mute-toggle` |
//! | **编曲视图**轨道头 | `ui/workspace/arrangement_view.slint` | `root.track-mute-toggle(track_index)` → `app.slint` 转给 `root.mixer-mute-toggle(track_index)` |
//!
//! ⇒ 在编曲视图点一下，撤销树的节点却声称那是"混音台"的动作。这条标签对用户是**假话**
//! （`docs/ledger/integration-rulings-notes.md` 的 R17 行逐字记录了它）。
//!
//! ## 处置：改中性文本（去掉来源前缀）
//!
//! 另一条可选处置是"给编曲视图自己的宿主回调"（两条回调 + 两条标签）。它需要新增
//! `MainWindow` 回调与两条 `wire_*` 分支，而收益只是把来源写进标签 ⇒ 本票选**最小**处置：
//! 标签不带来源，于是在两个入口下都成立。带来源的措辞要等宿主能**分别**知道入口
//! （那需要两条回调，本票不做，也不在这里假装做了）。
//!
//! ## 为什么判据住在 `tests/` 而不是 `src/host.rs` 的单元测试里
//!
//! 与 `tests/undo_tree_honesty.rs` 同款理由：`src/host.rs` 的 `mod tests` 是多线并发下的
//! 热点，判据放在**新文件**里 ⇒ 两个写者的改动面不重叠。

use std::path::PathBuf;

/// `crates/yeban-app/`。
fn crate_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 读 crate 内一个相对路径的文本。
fn read(rel: &str) -> String {
    let path = crate_dir().join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("读 {} 失败: {error}", path.display()))
}

/// 去掉 `//` 行注释（含行尾注释）之后的**代码文本**（口径与 `tests/undo_tree_honesty.rs` 同款）。
///
/// 为什么必须去掉：本票把"改掉了什么"写进注释留痕（含 `"mixer: …"` 这个旧字面量）。
/// 不去注释的话，留痕自己会把探针弄红，于是作者被迫删掉史实 —— 与本仓库的纪律相反。
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

/// `source` 里 `from` 与 `to` 之间的那一段。两个锚点都必须存在（缺一个就 panic）。
fn block<'a>(source: &'a str, from: &str, to: &str) -> &'a str {
    let start = source
        .find(from)
        .unwrap_or_else(|| panic!("源码里找不到起始锚点 `{from}`"));
    let rest = &source[start + from.len()..];
    let end = rest
        .find(to)
        .unwrap_or_else(|| panic!("起始锚点 `{from}` 之后找不到结束锚点 `{to}`"));
    &rest[..end]
}

/// `src/**/*.rs` 的全部**代码**文本（去掉 `//` 注释后拼接；单位：一个字符串）。
fn all_src_rs() -> String {
    fn walk(dir: &std::path::Path, out: &mut String) {
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

/// 一条**带来源视图名**的撤销标签（形状：`"<视图名>: <动作>"`）。
///
/// 它是一条代理指标：本判据只把它当"缺陷的字面形态"来拦。视图名取自本仓库真实的视图名集合
/// （`mixer` 是缺陷里那一个；其余几个是同一形状的邻居）—— 用一个"短前缀"的启发式会误伤
/// `"pencil: add note"` 这类**今天仍然是真话**的标签（卷帘编辑只有一个入口）。
fn names_a_view(literal: &str) -> bool {
    /// 本仓库的视图名（撤销标签里出现它们就是"把来源写进了历史"）。
    const VIEW_NAMES: [&str; 5] = ["mixer", "arrangement", "session", "console", "workspace"];
    match literal.split_once(": ") {
        Some((prefix, tail)) => !tail.is_empty() && VIEW_NAMES.contains(&prefix),
        None => false,
    }
}

// =========================================================================
// ① 两个开关的提交消息不得指明来源视图
// =========================================================================

/// 判据（**读实现里的字面量 + 入口的数量**）。
///
/// 四条子路径：
/// 1. `wire_mixer_switch` 的两个 match 臂各自给出一条 `commit_ops` 消息，两条都必须**不是**
///    `"<视图>: <动作>"` 形状（缺陷的字面形态）—— 去掉来源前缀之后，这条标签在两个入口下都成立；
/// 2. 两个标签必须**逐字不同**（静音与独奏是两条不同的历史，共用一条会把两者混成一件事）；
/// 3. 每条标签必须带上它真正改的字段名（`mute` / `solo`）—— 中性不等于含糊，撤销树要让用户
///    看出这一步改了什么；
/// 4. 两个入口**今天真的都在**：`arrangement_view.slint` 的轨道头点击经 `app.slint` 转到
///    **同一个** `mixer-mute-toggle` / `mixer-solo-toggle`。没有这一条，本判据钉的是一条
///    不存在的冲突（"假问题"）。
///
/// ## 代理指标的局限（如实说出）
///
/// 路径 1 是**文本**判据：它拦得住"把 `mixer:` 写回去"，但拦不住"换成另一个同样错的来源名"
/// （`names_a_view` 已按本仓库真实的视图名集合封口，因此换名也要换成一个已知视图名才会变绿）。
/// 它**拦不住**一个从未出现过的来源名（例如 `"editor: toggle track mute"`）——
/// 那需要判据知道"哪些词算来源"，而这个词表只能来自本仓库，不可能穷尽。
/// 路径 4（两个入口都在）因此是这一条的**结构性**补充：它证明错位的根因**今天仍然存在**，
/// 而不是靠文本形状猜出来的。
#[test]
fn the_two_mixer_toggle_labels_do_not_name_a_view() {
    let host = code_only(&read("src/host.rs"));
    // 结束锚点必须是**代码**行：`code_only` 去掉了全部 `//` 注释（含文档注释），
    // 因此 `/// …` 形状的锚点在它上面永远找不到。
    let wire = block(&host, "fn wire_mixer_switch(", "fn read_strings(");

    // 两个臂的消息是**整行就是那一个字面量**的两行（`"…",` 形状）。
    let labels: Vec<&str> = wire
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix('"')?;
            let (literal, tail) = rest.split_once('"')?;
            if tail.trim() == "," {
                Some(literal)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        labels.len(),
        2,
        "`wire_mixer_switch` 今天必须恰好给出两条提交消息（静音 / 独奏），实际 {labels:?}"
    );
    assert_ne!(
        labels[0], labels[1],
        "静音与独奏必须各自有一条标签（共用一条会把两条不同的历史混成一件事）"
    );

    for label in &labels {
        assert!(
            !names_a_view(label),
            "撤销树的节点标签不得带**来源视图**前缀（缺陷的字面形态是 `\"mixer: …\"`）：{label:?}\n{wire}"
        );
    }
    assert!(
        labels.iter().any(|label| label.contains("mute")),
        "静音开关的标签必须带上它真正改的字段名 `mute`（中性不等于含糊）: {labels:?}"
    );
    assert!(
        labels.iter().any(|label| label.contains("solo")),
        "独奏开关的标签必须带上它真正改的字段名 `solo`（中性不等于含糊）: {labels:?}"
    );

    // ---- 路径 4：两个界面入口今天都在（否则本判据钉的是一条不存在的冲突） ----
    let arrangement = code_only(&read("ui/workspace/arrangement_view.slint"));
    for (callback, host_callback) in [
        ("track-mute-toggle(int)", "mixer-mute-toggle"),
        ("track-solo-toggle(int)", "mixer-solo-toggle"),
    ] {
        assert!(
            arrangement.contains(&format!("callback {callback};")),
            "编曲视图必须声明 `callback {callback};` —— 它是与调音台并存的**第二个**入口，\
             也是这条标签错位的根因"
        );
        assert!(
            arrangement.contains(&format!("root.{}", callback.split('(').next().unwrap())),
            "编曲视图的 `{}` 必须真的有人调用（声明了不调用 = 第二个入口不存在）",
            callback.split('(').next().unwrap()
        );
        assert!(
            code_only(&read("ui/app.slint")).contains(host_callback),
            "编曲视图的轨道头开关必须转发到既有的 `{host_callback}`（`app.slint` 的那条转发）"
        );
    }
}

// =========================================================================
// ② `src/**` 里不许再有带来源前缀的图标标签
// =========================================================================

/// 判据（**全 `src/**` 的代码文本**）。
///
/// 这是对"只改两处"的**兜底**：即便作者把这两条修好了，另一个新写法（例如
/// `"arrangement: toggle track mute"`）也会以同一种方式骗过用户。判据按**旧字面量**
/// 逐条封口：两个旧标签不得回到 `src/**` 的代码里（注释里的留痕不算 —— `code_only` 去掉了）。
///
/// 为什么只封旧字面量而不是"封一切 `"…: …"`"：`"pencil: add note"` / `"mixer: set track volume"`
/// 这些**今天仍然是真话**（它们各自只有一个入口：卷帘编辑 / 调音台推子）。一刀切会把它们
/// 一起判红，而正确的处理是"等它们各自出现第二个入口时再改" —— 那属于另一票。
#[test]
fn the_old_source_prefixed_toggle_labels_are_gone_from_src() {
    let src = all_src_rs();
    for stale in [
        "\"mixer: toggle track mute\"",
        "\"mixer: toggle track solo\"",
    ] {
        assert!(
            !src.contains(stale),
            "`src/**` 的代码里不得再有带来源前缀的旧标签 {stale} —— 它是 R17 的缺陷字面量\
             （编曲视图点击后标签仍写 \"mixer:\"）"
        );
    }
}
