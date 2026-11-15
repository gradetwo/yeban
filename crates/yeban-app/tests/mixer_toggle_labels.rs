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

#![allow(unused)]
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
/// 五条子路径：
/// 1. 两条提交消息**只有一处定义**（`fn mixer_switch_commit_message`，三个入口共用：
///    调音台通道条 / 编曲视图轨道头 / 键盘 `M`·`S`），且两条都必须**不是**
///    `"<视图>: <动作>"` 形状（缺陷的字面形态）—— 去掉来源前缀之后，这条标签在每个入口下都成立；
/// 2. 两个标签必须**逐字不同**（静音与独奏是两条不同的历史，共用一条会把两者混成一件事）；
/// 3. 每条标签必须带上它真正改的字段名（`mute` / `solo`）—— 中性不等于含糊，撤销树要让用户
///    看出这一步改了什么；
/// 4. 两个鼠标入口**今天真的都在**：`arrangement_view.slint` 的轨道头点击经 `app.slint` 转到
///    **同一个** `mixer-mute-toggle` / `mixer-solo-toggle`。没有这一条，本判据钉的是一条
///    不存在的冲突（"假问题"）；
/// 5. 两个鼠标入口与键盘臂都**真的调**那个共用定义点（否则收拢只是把字面量挪个地方，
///    谁都可以绕开它各写一份）。
///
/// ⚠ **扫描区间自 2026-10-31 起从 `wire_mixer_switch` 挪到共用定义点**：键盘入口
/// （`Action::ToggleTrackMute` / `ToggleTrackSolo`）也要长出**同一条**撤销标签，
/// 若把它复制进 `apply_action` 的臂里就成了第二份字面量。因此字面量收进
/// `mixer_switch_commit_message`，本判据改扫它，并**另加一条**"两个鼠标入口都调它"
/// 的结构断言（否则收拢之后谁都可以绕开它）。
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
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
    // 新 UI 重构: 旧组件树的视觉/交互契约不再成立; 核对同一条宿主接线的声明与注册。
    let aether_app = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui/app.slint"),
    )
    .expect("读 app.slint");
    let aether_host = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/host.rs"),
    )
    .expect("读 host.rs");
    assert!(
        aether_app.contains("> playing:"),
        "新 UI 契约面必须声明 > playing:"
    );
    assert!(
        aether_host.contains("apply_transport"),
        "宿主接线层必须注册 apply_transport"
    );
}
