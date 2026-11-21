//! 输入法合成态的**界面侧接线**（审计 `#20`）—— 文本层判据。
//!
//! ## 本文件要证的句子
//!
//! 病根不是"状态机写错了"，而是**事件源根本不存在**：`MainWindow` 早就声明了
//! `ime-composition-changed` / `ime-focus-changed`，宿主 `host::wire_input` 也早就接着，
//! 但全仓唯一写 `changed preedit-text` 的地方是 `ui/transport.slint` 的 `bpm-input`，
//! 而那个文件**不在** `app.slint` 的 import 图里（`build.rs` 只编译 `ui/app.slint`
//! 一个根）⇒ 回调永远没人调，CJK 组字期间敲下的字母照旧冒泡到 `key-handler`，
//! 触发了单键热键（`x` 切混音台、`e` 开编辑器 …）。
//!
//! 因此本片钉的不是"合成态语义"（那是 `src/input.rs` 的 22 条判据，已在
//! `composing_swallows_every_bare_shortcut` 等处覆盖），而是**链路上有没有断点**。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（唯一汇聚点） | `ui/tokens.slint` 的 `ImeState` 同时给出**当前值**（两个属性）与**变化**（两个回调）；`ImeAwareTextInput` 继承内置 `TextInput` 且两条 `changed` 都搬 |
//! | ②（没有裸框回流） | `app.slint` 可达的 9 个文件里 `TextInput {` **零**出现、`ImeAwareTextInput {` 恰好 21 个（审计口径：23 处声明 − 未编译的 `transport.slint` 那 1 处 − 已被 `left_panel.slint` 取代、无人 import 的 `sidebar.slint` 那 1 处） |
//! | ③（宿主直连 global） | `host.rs` 注册的是 `ImeState.on_composition_changed` / `on_focus_changed`（Slint 1.18.1 的连接语句只接受元素 id，别名已弃用） |
//! | ④（门放在交接之后） | `key-handler` 里 `ImeState.composing` 的判断在 `root.trigger-shortcut` **之后** —— 否则 F5/F6 会被连坐吞掉（`resolve_logical` 第 1 步先结算它们） |
//!
//! ## 为什么是文本层而不是像素层
//!
//! 本线**没有视觉改动**（`ImaAwareTextInput` 继承同一个内置元素，字体 / 颜色 / 尺寸
//! 由 22 个调用点各自显式写死），所以像素类验收不适用；而"某个 `changed` 有没有人
//! 写"这件事在运行时**看不出来**（没人写的回调与没人读的回调一样安静），只能在
//! 源码层钉。手工自验路径见交付回执。

use std::fs;
use std::path::{Path, PathBuf};

/// `crates/yeban-app` 根。
fn crate_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn read_ui(rel: &str) -> String {
    let path = crate_root().join("ui").join(rel);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()))
}

fn read_src(rel: &str) -> String {
    let path = crate_root().join("src").join(rel);
    fs::read_to_string(&path).unwrap_or_else(|err| panic!("读不到 {}: {err}", path.display()))
}

/// 去掉 `//` 行注释与 `/* */` 块注释，只留下真正的代码行。
///
/// 判据全在"代码里有没有这句话"，而本仓的注释密度极高（每个 `TextInput` 周围都有
/// 一段考据），不去注释会让"注释里提到 `TextInput {`"变成假绿或假红。
fn strip_line_comments(text: &str) -> String {
    text.lines()
        .map(|line| match line.find("//") {
            Some(cut) => &line[..cut],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `app.slint` 的 import 图可达的 `.slint` 文件（递归）。
///
/// ⛔ 不写死清单：写死就证不出"新加的文件也被覆盖"，而本判据的价值正是**普查**。
fn reachable_slint(root: &Path) -> Vec<PathBuf> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut stack: Vec<PathBuf> = vec![root.join("ui/app.slint")];
    while let Some(current) = stack.pop() {
        if seen.contains(&current) {
            continue;
        }
        let Ok(text) = fs::read_to_string(&current) else {
            continue;
        };
        for line in text.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("import") {
                continue;
            }
            let Some(from) = trimmed.find("from ") else {
                continue;
            };
            let tail = trimmed[from + 5..].trim_start();
            let Some(rest) = tail.strip_prefix('"') else {
                continue;
            };
            let Some(close) = rest.find('"') else {
                continue;
            };
            let target = current
                .parent()
                .map(|dir| dir.join(&rest[..close]))
                .unwrap_or_default();
            if target.extension().is_some_and(|ext| ext == "slint") {
                stack.push(target);
            }
        }
        seen.push(current);
    }
    seen
}

/// ① 单一汇聚点：`ImeState` 的四个成员 + 组件的两条 `changed`。
#[test]
fn ime_state_is_the_single_funnel() {
    let tokens = strip_line_comments(&read_ui("tokens.slint"));

    // 当前值（`key-handler` 要读）与变化（宿主收）**并存**：Slint 1.18.1 的 `changed`
    // 只观察本组件属性，`changed ImeState.x` 是语法错误（实测 `expected '=>'`），
    // 所以"变化"只能由**写它的地方**发回调。
    for needle in [
        "export global ImeState {",
        "in-out property <bool> composing: false;",
        "in-out property <bool> text-focused: false;",
        "callback composition-changed(bool);",
        "callback focus-changed(bool);",
    ] {
        assert!(
            tokens.contains(needle),
            "ImeState 缺少 `{needle}` —— 汇聚点必须同时提供当前值与变化"
        );
    }

    // 组件必须是**继承**内置 `TextInput`，而不是套一层壳：属性 / 事件 / 无障碍角色
    // 全部原样保留，22 个调用点才能只换类型名。
    assert!(
        tokens.contains("export component ImeAwareTextInput inherits TextInput {"),
        "ImeAwareTextInput 必须 `inherits TextInput`（换壳会丢 accessible_role 与全部属性）"
    );
    for needle in [
        "changed preedit-text =>",
        "ImeState.composing = self.preedit-text != \"\";",
        "ImeState.composition-changed(ImeState.composing);",
        "changed has-focus =>",
        "ImeState.text-focused = self.has-focus;",
        "ImeState.focus-changed(self.has-focus);",
    ] {
        assert!(
            tokens.contains(needle),
            "ImeAwareTextInput 缺少 `{needle}` —— 合成态 / 焦点态漏一条就静默失效"
        );
    }
}

/// ② 可达文件里**没有裸 `TextInput`**，且替换数是 22。
#[test]
fn every_reachable_text_input_is_ime_aware() {
    let reachable = reachable_slint(&crate_root());
    assert!(
        reachable.len() >= 20,
        "import 图解析只找到 {} 个文件 —— 解析器本身失效了，不是判据通过",
        reachable.len()
    );

    let mut bare: Vec<String> = Vec::new();
    let mut aware = 0_usize;
    for path in &reachable {
        let Ok(text) = fs::read_to_string(path) else {
            continue;
        };
        let code = strip_line_comments(&text);
        // `reachable_slint` 落的是 `join` 出来的路径，会带 `dialogs/../tokens.slint`
        // 这类未归一化的段；先规范化，才能正确识别「定义处」这一个文件。
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        let ui_root = crate_root().join("ui");
        let ui_root = ui_root.canonicalize().unwrap_or(ui_root);
        let name = canonical
            .strip_prefix(&ui_root)
            .unwrap_or(&canonical)
            .display()
            .to_string();
        // `tokens.slint` 是**定义处**（`inherits TextInput` + 组件体），不参与替换统计。
        if name.ends_with("tokens.slint") {
            continue;
        }
        // ⚠ `"ImeAwareTextInput {"` **包含**子串 `"TextInput {"`，直接 `matches` 会把
        // 已替换的 21 处全判成裸控件。先摘掉前缀再数剩下的。
        let without_aware = code.replace("ImeAwareTextInput {", "");
        let bare_count = without_aware.matches("TextInput {").count();
        if bare_count > 0 {
            bare.push(format!("{name}: {bare_count}"));
        }
        aware += code.matches("ImeAwareTextInput {").count();
    }

    assert!(
        bare.is_empty(),
        "这些可达文件里还有裸 `TextInput`（组字期间按键会漏到热键层）: {bare:?}"
    );
    // 口径自洽：那两个被扣掉的文件必须**确实**不在 import 图里。哪天有人把
    // `sidebar.slint` 接回 `app.slint`，这里的数字就会自己变红，而不是悄悄少统计一个框。
    let reachable_names: Vec<String> = reachable
        .iter()
        .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .collect();
    for excluded in ["transport.slint", "sidebar.slint"] {
        assert!(
            !reachable_names.iter().any(|n| n == excluded),
            "`{excluded}` 已进入 `app.slint` 的 import 图 —— 口径里的 −1 不再成立，请连同上面的实例数一起更新"
        );
    }
    assert_eq!(
        aware, 21,
        "`ImeAwareTextInput` 实例数必须是 21（23 处声明 − 未编译的 `transport.slint` 1 处 \
         − 已被 `left_panel.slint` 取代且无人 import 的 `sidebar.slint` 1 处），实际 {aware}"
    );
}

/// ③ 宿主直连 global；旧的一次性回调名不再被界面层使用。
#[test]
fn host_binds_the_global_not_a_dead_callback() {
    let host = strip_line_comments(&read_src("host.rs"));
    assert!(
        host.contains(".on_composition_changed("),
        "`wire_input` 必须注册 `ImeState.on_composition_changed`（22 个输入框的唯一生产事件源）"
    );
    assert!(
        host.contains(".on_focus_changed("),
        "`wire_input` 必须注册 `ImeState.on_focus_changed`（`InputContext::set_focus` 的唯一来源）"
    );
    // 合成本身**只实现一次**：global 那条只调 `invoke_ime_composition_changed`，
    // 落点仍是 `begin_composition` / `end_composition`。
    assert!(
        host.contains("ui.invoke_ime_composition_changed(composing_now)"),
        "global 的合成事件必须**委托**给既有回调，⛔ 不是第二份 begin/end_composition"
    );
    // 焦点**不许**复用 BPM 那条：那会把 `bpm-field-editing` 置真、冻住走带读数。
    assert!(
        !host.contains("on_ime_text_focus_changed"),
        "焦点态⛔ 不走 `MainWindow` 上新增的回调 —— 直连 `ImeState` 即可（少一圈界面层管道）"
    );
}

/// ④ 合成态的门在 `trigger-shortcut` **之后**。
#[test]
fn composition_gate_sits_after_the_shortcut_handoff() {
    let app = strip_line_comments(&read_ui("app.slint"));
    let handoff = app
        .find("root.trigger-shortcut(event.text, event);")
        .expect("`key-handler` 必须先把手键交给宿主（`wire_keys` → `input.rs`）");
    let gate = app
        .find("if (ImeState.composing) {")
        .expect("`key-handler` 必须在合成态收手（否则 i / h / t / F3 仍会翻面板）");
    assert!(
        handoff < gate,
        "合成态判断必须排在 `trigger-shortcut` **之后**：它前面还有 F5/F6 这条\
         「全局无冲突备选快捷键」（`resolve_logical` 第 1 步就结算），抢先 return 会把它们连坐吞掉"
    );
    // 收手必须是"已经交给宿主之后再收"，所以这两句之间只有注释与空行。
    let window = &app[handoff..gate];
    assert!(
        !window.contains("return "),
        "`trigger-shortcut` 与合成态判断之间⛔ 不许有别的 `return`（会把门架空）"
    );
}
