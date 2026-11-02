//! **界面上印出来的和弦，必须是真绑的那一把**（缺口 `D-UI-26`）—— 端到端判据。
//!
//! ## 本文件要证的句子
//!
//! 一行**印给用户看的键位提示**（`.slint` 里的 `accessible-label` / 可见 `text`，以及
//! `src/elements.rs` 登记名里带的那一份）是对用户的**承诺**：屏读器会念它，控制面会把
//! 它交给 AI，用户会照着按。承诺的每一把和弦都必须能被 `src/input.rs` 的那**一张**键表
//! 解析成一个动作 —— 念出来却什么都不会发生的键，与"按钮点了没反应"同罪，而且更难发现
//! （⛔ 有像素、⛔ 有回调、只有一行文案）。
//!
//! 本票抓到的那一处：`ui/transport.slint` 的 `transport-ai-drawer-button` 曾写
//! `"AI 协作栏抽屉 (Cmd/Ctrl + 反斜杠)"`，而键表的 `command()` 分支里**从未**有过 `'\\'`
//! 那一臂（`Cmd/Ctrl+\` ⇒ `_ => PassThrough`）；AI 抽屉真正的键盘入口是
//! `Cmd/Ctrl+Alt+A`（`ADR-0064` 第 4 条点名，端到端判据 = `tests/ai_drawer_key_ui.rs`）。
//! ⇒ 修法是⭐ 把文案改成真绑的那一把，⛔ 是给 `\` 补一条臂（那会造出**第二把**键干同一件事）。
//!
//! ## 两个承诺源，一套解析器
//!
//! `src/elements.rs` 的 `registry.add(id, kind, file, name, ..)` 里那份 `name` 是**第二个**
//! 承诺源（`yeban-ui-test-port` 的控件树读数，屏读与 AI 都从这里取键位）。实测今天它有三格
//! 携带和弦（`transport-metronome-button` / `musical-pr-accept-button` / `undo-tree-undo-button`），
//! 且逐字等于对应 `.slint` 里的那条字面 ⇒ 本守卫把两边**一起**扫，并把"不许漂移"钉成 ⑤。
//! Rust 侧只扫 `registry.add(..)` 的参数区间：`elements.rs` 的**说明文字**里也提到
//! `Shift+Enter` 这类串（实测第 3964 行那条 assert 消息），把散文当承诺会让守卫变成假红机器。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（清点） | 两个承诺源扫出的和弦**多重集**恰好等于钉值清单（原话＋次数都钉住 ⇒ 解析器⛔ 自嗨；新增承诺必须同步扩这张表） |
//! | ②（兑现） | 每一把和弦喂给 `InputContext::resolve_logical`（画布焦点）都解析出**期望的那个** `Action`；`Cmd/Ctrl` 一律**两种形态都验**（`Modifiers::command()` 就是 `ctrl` 或 `meta`） |
//! | ③（反面事实） | `Cmd/Ctrl + \\` 在键表里 ⇒ `PassThrough`，且扫出的和弦⛔ 含反斜杠键 ⇒ 那句假承诺⛔ 许回来 |
//! | ④（本票那一格） | AI 抽屉按钮的 label 逐字 = 真绑的那把；登记名那一格⛔ 携带任何和弦（不带 ⇒ ⛔ 是第二处假承诺） |
//! | ⑤（两源不漂移） | 携带和弦的 `registry.add` `name` 必须与某条 `.slint` 字面**逐字相同** ⇒ 界面与控件树不会各说一把键 |
//! | ⑥（扫描面完整） | 磁盘上真实存在的 `.slint` 文件必须都在**编译期嵌入**的清单里（口径同 `tests/assertion_bounds_audit.rs`：主体扫描用 `include_str!`，`read_dir` 只做完整性对账） |
//! | ⑦（承诺写全） | 字符串字面里出现修饰词却没有 `+ 键名` ⇒ 红；出现词表不认识的键名 ⇒ 红（⛔ 的口径是"认不出就当它不是承诺"，那会静默漏网） |
//! | ⑧（上下文口径） | 同一把和弦在 `Focus::TextInput` 下必须仍 ⇒ `PassThrough` —— 文本框里 DAW 热键整体放行是**现状**（`ADR-0067` 未决），本守卫⛔ 把它算成"承诺兑现" |
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! 本文件⛔ 装配 Slint 平台（纯源码扫描 ＋ 纯逻辑判据，⛔ 活窗口）⇒ 多个 `#[test]` 自由；
//! 但它是 `yeban-app` 的判据 ⇒ 编译与判决交 CI。

use std::collections::{BTreeMap, BTreeSet};

use yeban_app::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution};

/// 承诺源 A：界面源，**编译期固定**（`include_str!`）。
///
/// ⚠ 口径来自 `tests/assertion_bounds_audit.rs`：主体扫描⛔ 用 `read_dir` —— 目录读不到
/// 或文件缺失会**空转成绿**；`read_dir` 只留给 ⑥ 那条完整性对账。
const SLINT_SOURCES: &[(&str, &str)] = &[
    ("ui/app.slint", include_str!("../ui/app.slint")),
    (
        "ui/console/asset_pool_panel.slint",
        include_str!("../ui/console/asset_pool_panel.slint"),
    ),
    (
        "ui/console/console_tabs.slint",
        include_str!("../ui/console/console_tabs.slint"),
    ),
    (
        "ui/console/device_rack.slint",
        include_str!("../ui/console/device_rack.slint"),
    ),
    (
        "ui/console/mixer_console.slint",
        include_str!("../ui/console/mixer_console.slint"),
    ),
    (
        "ui/console/piano_roll.slint",
        include_str!("../ui/console/piano_roll.slint"),
    ),
    (
        "ui/dialogs/export_wav_dialog.slint",
        include_str!("../ui/dialogs/export_wav_dialog.slint"),
    ),
    (
        "ui/dialogs/macro_panel.slint",
        include_str!("../ui/dialogs/macro_panel.slint"),
    ),
    (
        "ui/dialogs/musical_pr_drawer.slint",
        include_str!("../ui/dialogs/musical_pr_drawer.slint"),
    ),
    (
        "ui/dialogs/undo_tree_modal.slint",
        include_str!("../ui/dialogs/undo_tree_modal.slint"),
    ),
    (
        "ui/module_actions.slint",
        include_str!("../ui/module_actions.slint"),
    ),
    ("ui/sidebar.slint", include_str!("../ui/sidebar.slint")),
    (
        "ui/status_bar.slint",
        include_str!("../ui/status_bar.slint"),
    ),
    ("ui/tokens.slint", include_str!("../ui/tokens.slint")),
    ("ui/transport.slint", include_str!("../ui/transport.slint")),
    (
        "ui/workspace/arrangement_view.slint",
        include_str!("../ui/workspace/arrangement_view.slint"),
    ),
    (
        "ui/workspace/session_view.slint",
        include_str!("../ui/workspace/session_view.slint"),
    ),
];

/// 承诺源 B：控件登记表（`registry.add(id, kind, file, name, ..)`）。
const REGISTRY_FILE: &str = "src/elements.rs";
const REGISTRY_SOURCE: &str = include_str!("../src/elements.rs");

/// 登记一格的起始标记：只有落在 `registry.add(..)` 参数区间里的字面才算承诺源 B。
const REGISTRY_ADD_MARK: &str = "registry.add(";

/// 和弦里认识的修饰词。⚠ **必须按声明顺序**（最长优先）匹配：`Cmd/Ctrl` 要抢在 `Cmd` 前面。
const MODIFIER_TOKENS: &[(&str, ModifierFlag)] = &[
    ("Cmd/Ctrl", ModifierFlag::Command),
    ("Ctrl/Cmd", ModifierFlag::Command),
    ("Command", ModifierFlag::Command),
    ("⌘", ModifierFlag::Command),
    ("Cmd", ModifierFlag::Command),
    ("Ctrl", ModifierFlag::Command),
    ("Option", ModifierFlag::Alt),
    ("⌥", ModifierFlag::Alt),
    ("Alt", ModifierFlag::Alt),
    ("Shift", ModifierFlag::Shift),
];

/// 键名词表（多字符名，以及 `↑` 这类单字符但⛔ 是 ASCII 字母的键名）。
/// ⚠ 单字符的 ASCII 字母/数字由 `key_of` 的另一条腿收。
const KEY_WORDS: &[(&str, LogicalKey)] = &[
    ("反斜杠", LogicalKey::Character('\\')),
    ("空格", LogicalKey::Space),
    ("Tab", LogicalKey::Tab),
    ("回车", LogicalKey::Enter),
    ("Enter", LogicalKey::Enter),
    ("Esc", LogicalKey::Escape),
    ("Escape", LogicalKey::Escape),
    ("退格", LogicalKey::Backspace),
    ("Backspace", LogicalKey::Backspace),
    ("删除", LogicalKey::Delete),
    ("Del", LogicalKey::Delete),
    ("Delete", LogicalKey::Delete),
    ("F5", LogicalKey::F5),
    ("F6", LogicalKey::F6),
    ("F7", LogicalKey::F7),
    ("F8", LogicalKey::F8),
    ("F9", LogicalKey::F9),
    ("↑", LogicalKey::UpArrow),
    ("↓", LogicalKey::DownArrow),
    ("←", LogicalKey::LeftArrow),
    ("→", LogicalKey::RightArrow),
];

/// 键名 token 的终止字符集：空白、成对括号、中点（`·` 是仓库里分隔提示的写法）、引号、分号。
const KEY_STOPS: &[char] = &[' ', '(', ')', '·', '"', ';'];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ModifierFlag {
    /// `Cmd`（mac）或 `Ctrl`（其余平台）—— 键表问的是 `Modifiers::command()`，两种形态都验。
    Command,
    Alt,
    Shift,
}

/// 一把被界面**印出来**的和弦。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chord {
    /// `ui/transport.slint` / `src/elements.rs` 这样的仓库内相对路径。
    file: &'static str,
    line: usize,
    /// 原话切片（**照抄源文件**，连分隔空格都按原文：`Cmd+Z` 不是 `Cmd + Z`）—— ① 的钉值比这个。
    printed: String,
    /// 装着这把和弦的**整条**字符串字面（⑤ 要比逐字相同）。
    literal: String,
    command: bool,
    alt: bool,
    shift: bool,
    /// 解析出的逻辑键（② 喂给键表的就是它）。
    key: LogicalKey,
}

/// 一处"提到了键、却没构成一把完整和弦"的字面（⑦ 的对象）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Occurrence {
    file: &'static str,
    line: usize,
    printed: String,
}

#[derive(Debug, Default)]
struct Evidence {
    chords: Vec<Chord>,
    /// 修饰词后面接不到 `+ 键名`（"需要 Ctrl 键"这类散文，或写坏了的承诺）。
    incomplete: Vec<Occurrence>,
    /// 接到了键名，但键名不在词表里 ⇒ ⛔ 当它不是承诺（那是静默漏网）。
    unknown_keys: Vec<Occurrence>,
}

/// 一条字符串字面在源文件里的位置。
#[derive(Debug)]
struct Literal {
    line: usize,
    text: String,
    /// `Some(n)` ⇒ 这条字面落在第 n 次 `registry.add(` 的参数区间里。
    add_span: Option<usize>,
}

impl Evidence {
    fn absorb(&mut self, other: Evidence) {
        self.chords.extend(other.chords);
        self.incomplete.extend(other.incomplete);
        self.unknown_keys.extend(other.unknown_keys);
    }

    /// `原话 -> 出现次数`（① 的多重集）。
    fn counts(&self) -> BTreeMap<&str, usize> {
        let mut map: BTreeMap<&str, usize> = BTreeMap::new();
        for chord in &self.chords {
            *map.entry(chord.printed.as_str()).or_insert(0) += 1;
        }
        map
    }
}

fn starts_with_at(chars: &[char], at: usize, token: &str) -> bool {
    let needle: Vec<char> = token.chars().collect();
    chars[at..].starts_with(needle.as_slice())
}

fn find_modifier(chars: &[char]) -> Option<(&'static str, ModifierFlag)> {
    MODIFIER_TOKENS
        .iter()
        .find(|(token, _)| starts_with_at(chars, 0, token))
        .map(|(token, flag)| (*token, *flag))
}

fn is_key_stop(c: char) -> bool {
    KEY_STOPS.contains(&c)
}

fn key_of(token: &str) -> Option<LogicalKey> {
    if let Some((_, key)) = KEY_WORDS.iter().find(|(name, _)| *name == token) {
        return Some(*key);
    }
    let mut it = token.chars();
    let (Some(first), None) = (it.next(), it.next()) else {
        return None;
    };
    let lower = first.to_lowercase().next()?;
    if lower.is_ascii_alphanumeric() {
        return Some(LogicalKey::Character(lower));
    }
    None
}

/// 扫出源文件里**代码中**（⛔ 注释里的）字符串字面，并标出哪些落在 `registry.add(..)` 区间内。
///
/// ⛔ 整行数 `"`：`.slint` 与 `elements.rs` 的注释里满是引号里的和弦（本文件自己的文档
/// 就是一例），把注释算成承诺会把守卫变成假红机器。
fn literals(source: &str) -> Vec<Literal> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let mut line = 1usize;
    let mut i = 0usize;
    let mut add_span: Option<usize> = None;
    let mut spans_seen = 0usize;
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if c == '/' && starts_with_at(&chars, i, "//") {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && starts_with_at(&chars, i, "/*") {
            i += 2;
            let mut depth = 1usize;
            while i < chars.len() && depth > 0 {
                if starts_with_at(&chars, i, "/*") {
                    depth += 1;
                    i += 2;
                } else if starts_with_at(&chars, i, "*/") {
                    depth -= 1;
                    i += 2;
                } else {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if starts_with_at(&chars, i, REGISTRY_ADD_MARK) {
            spans_seen += 1;
            add_span = Some(spans_seen);
            i += REGISTRY_ADD_MARK.chars().count();
            continue;
        }
        if c == '"' {
            let start_line = line;
            i += 1;
            let mut buf = String::new();
            while i < chars.len() {
                let d = chars[i];
                if d == '\\' {
                    // `\` 之后的字符**照抄**：转义序列留在原文里（`"\""` 不该提前终结字面）。
                    buf.push(d);
                    i += 1;
                    if i < chars.len() {
                        if chars[i] == '\n' {
                            line += 1;
                        }
                        buf.push(chars[i]);
                        i += 1;
                    }
                    continue;
                }
                if d == '"' {
                    i += 1;
                    break;
                }
                if d == '\n' {
                    line += 1;
                }
                buf.push(d);
                i += 1;
            }
            out.push(Literal {
                line: start_line,
                text: buf,
                add_span,
            });
            continue;
        }
        if c == ')' && add_span.is_some() {
            add_span = None;
        }
        i += 1;
    }
    out
}

/// 从**一段字符串字面**里扫出所有和弦承诺（与所有"写坏了的键位提示"）。
fn chords_in_literal(file: &'static str, line: usize, literal: &str) -> Evidence {
    let chars: Vec<char> = literal.chars().collect();
    let mut out = Evidence::default();
    let mut i = 0usize;
    while i < chars.len() {
        let Some((token, flag)) = find_modifier(&chars[i..]) else {
            i += 1;
            continue;
        };
        let mut command = flag == ModifierFlag::Command;
        let mut alt = flag == ModifierFlag::Alt;
        let mut shift = flag == ModifierFlag::Shift;
        let mut pos = i + token.chars().count();
        let mut parsed: Option<LogicalKey> = None;
        let mut unknown: Option<String> = None;
        loop {
            let mut p = pos;
            while chars.get(p) == Some(&' ') {
                p += 1;
            }
            if chars.get(p) != Some(&'+') {
                break;
            }
            p += 1;
            while chars.get(p) == Some(&' ') {
                p += 1;
            }
            if let Some((next_token, next_flag)) = find_modifier(&chars[p..]) {
                command |= next_flag == ModifierFlag::Command;
                alt |= next_flag == ModifierFlag::Alt;
                shift |= next_flag == ModifierFlag::Shift;
                pos = p + next_token.chars().count();
                continue;
            }
            let mut end = p;
            while chars.get(end).is_some_and(|c| !is_key_stop(*c)) {
                end += 1;
            }
            let token_str: String = chars[p..end].iter().collect();
            match key_of(&token_str) {
                Some(key) => parsed = Some(key),
                None => unknown = Some(token_str),
            }
            pos = end;
            break;
        }
        let printed: String = chars[i..pos].iter().collect();
        if let Some(key) = parsed {
            out.chords.push(Chord {
                file,
                line,
                printed,
                literal: literal.to_owned(),
                command,
                alt,
                shift,
                key,
            });
            i = pos.max(i + 1);
            continue;
        }
        if let Some(token_str) = unknown {
            out.unknown_keys.push(Occurrence {
                file,
                line,
                printed: format!("{printed}（词表不认识的键名 {token_str:?}）"),
            });
            i = pos.max(i + 1);
            continue;
        }
        out.incomplete.push(Occurrence {
            file,
            line,
            printed,
        });
        i += 1;
    }
    out
}

/// 两个承诺源的全部证据。
fn scan_corpus() -> Evidence {
    let mut out = Evidence::default();
    for (file, source) in SLINT_SOURCES {
        for literal in literals(source) {
            out.absorb(chords_in_literal(file, literal.line, &literal.text));
        }
    }
    for literal in literals(REGISTRY_SOURCE) {
        // 登记区间之外的一切（说明文字、assert 消息）⛔ 是对用户的承诺。
        if literal.add_span.is_none() {
            continue;
        }
        out.absorb(chords_in_literal(
            REGISTRY_FILE,
            literal.line,
            &literal.text,
        ));
    }
    out
}

/// `.slint` 一侧的全部字面原文（⑤ 用它比逐字相同）。
fn slint_literals() -> BTreeSet<String> {
    SLINT_SOURCES
        .iter()
        .flat_map(|(_, source)| literals(source))
        .map(|literal| literal.text)
        .collect()
}

/// 把 `registry.add(..)` 的每一格参数收集起来：`区间序号 -> 该格全部字面`。
fn registry_entries() -> BTreeMap<usize, Vec<String>> {
    let mut grouped: BTreeMap<usize, Vec<String>> = BTreeMap::new();
    for literal in literals(REGISTRY_SOURCE) {
        if let Some(span) = literal.add_span {
            grouped.entry(span).or_default().push(literal.text);
        }
    }
    grouped
}

/// 找到登记格里那一格的用户可读名（⛔ id、⛔ `.slint` 文件名）。
fn registry_name(id: &str) -> Option<String> {
    registry_entries()
        .values()
        .find(|texts| texts.first().is_some_and(|first| first == id))
        .and_then(|texts| {
            texts
                .iter()
                .find(|text| text.as_str() != id && !text.ends_with(".slint"))
                .cloned()
        })
}

/// 把一把和弦按它承诺的形态喂给键表（画布焦点）：`Cmd/Ctrl` 有 `Ctrl` 与 `Cmd` 两条腿，
/// 两条都必须命中 ⇒ 返回 `(形态名, 处置)` 的**全部**组合。
fn resolve_on_canvas(chord: &Chord) -> Vec<(&'static str, Resolution)> {
    let base = Modifiers {
        ctrl: false,
        shift: chord.shift,
        alt: chord.alt,
        meta: false,
    };
    let context = InputContext::new();
    if chord.command {
        vec![
            (
                "Ctrl",
                context.resolve_logical(chord.key, Modifiers { ctrl: true, ..base }),
            ),
            (
                "Cmd",
                context.resolve_logical(chord.key, Modifiers { meta: true, ..base }),
            ),
        ]
    } else {
        vec![("无修饰", context.resolve_logical(chord.key, base))]
    }
}

/// ①/② 的钉值清单：`(界面上印出来的原话, 次数, 期望动作)`。
///
/// ⚠ 这张表是**棘轮**：新增一处印出来的和弦，就必须在这里添一行（否则 ① 的多重集对账先红）。
/// 期望动作逐条现读 `src/input.rs` 的键表（第 3 步 `command()` 分支、第 5 步 `shift` 分支），
/// ⛔ 靠文案猜。次数按**两个承诺源**合并算：`elements.rs` 的登记名与 `.slint` 的 label 各一份。
const PINNED_PROMISES: &[(&str, usize, Action)] = &[
    ("Cmd/Ctrl + Alt + N", 2, Action::ToggleMetronome),
    ("Cmd/Ctrl + Alt + A", 1, Action::ToggleAiDrawer),
    ("Cmd+Z", 3, Action::Undo),
    ("Shift+Enter", 3, Action::AcceptAiSuggestion),
];

fn pinned_counts() -> BTreeMap<&'static str, usize> {
    PINNED_PROMISES
        .iter()
        .map(|(text, times, _)| (*text, *times))
        .collect()
}

fn expected_action(printed: &str) -> Option<Action> {
    PINNED_PROMISES
        .iter()
        .find(|(text, _, _)| *text == printed)
        .map(|(_, _, action)| *action)
}

/// ①（清点）：两个承诺源扫出的和弦**多重集**恰好是钉值清单。
#[test]
fn the_printed_chords_are_exactly_the_pinned_inventory() {
    let evidence = scan_corpus();
    assert!(
        !evidence.chords.is_empty(),
        "一把都没扫出来 ⇒ 扫描面失效（先看 ⑥ 的完整性对账，⛔ 把空集当成绿）"
    );
    assert_eq!(
        evidence.counts(),
        pinned_counts(),
        "界面上印出来的和弦多重集与钉值清单不符 ⇒ 新增的承诺要同步扩 PINNED_PROMISES\
         （原话与次数都算），⛔ 让新承诺没有判据"
    );
}

/// ②（承诺兑现）：每一把印出来的和弦都必须被键表解析成**期望的那个**动作。
#[test]
fn every_printed_chord_resolves_to_an_action() {
    let evidence = scan_corpus();
    for chord in &evidence.chords {
        let expected = expected_action(&chord.printed)
            .unwrap_or_else(|| panic!("扫出的和弦 {:?} 不在钉值清单里", chord.printed));
        let forms = resolve_on_canvas(chord);
        assert!(
            !forms.is_empty(),
            "{}:{} 的 `{}` 一条形态都没验 ⇒ 扫描器与键表的对接失效",
            chord.file,
            chord.line,
            chord.printed
        );
        for (form, resolution) in forms {
            assert_eq!(
                resolution,
                Resolution::Action(expected),
                "{}:{} 印着 `{}`（整条字面 {:?}）—— {form} 形态在键表里解析成 {resolution:?}，\
                 期望 `Action::{expected:?}` ⇒ 念出来却什么都不会发生",
                chord.file,
                chord.line,
                chord.printed,
                chord.literal
            );
        }
    }
}

/// ③（反面事实）：`Cmd/Ctrl + 反斜杠`⛔ 是任何人的承诺，因为它⛔ 被键表实现。
#[test]
fn the_backslash_chord_stays_unimplemented_and_unpromised() {
    let context = InputContext::new();
    for command_form in [false, true] {
        let modifiers = Modifiers {
            ctrl: command_form,
            shift: false,
            alt: false,
            meta: command_form,
        };
        assert_eq!(
            context.resolve_logical(LogicalKey::Character('\\'), modifiers),
            Resolution::PassThrough,
            "`Cmd/Ctrl + \\` 在键表里有了动作（形态 {command_form}）—— 那本票的修法要改写：\
             要么⭐ 这就是第二把干同一件事的键（得先按界面决定裁一遍），要么文案得跟着改"
        );
    }
    for chord in scan_corpus().chords {
        assert_ne!(
            chord.key,
            LogicalKey::Character('\\'),
            "{}:{} 又把 `Cmd/Ctrl + 反斜杠` 印成了对用户的承诺：{chord:?}",
            chord.file,
            chord.line
        );
    }
}

/// ④（本票那一格）：AI 抽屉按钮的 label 逐字 = 真绑的那把；登记名⛔ 带第二处承诺。
#[test]
fn the_ai_drawer_button_promises_the_bound_chord() {
    const LABEL: &str = "AI 协作栏抽屉 (Cmd/Ctrl + Alt + A)";
    const BUTTON_ID: &str = "transport-ai-drawer-button";
    const TRANSPORT: &str = "ui/transport.slint";

    let transport = SLINT_SOURCES
        .iter()
        .find(|(file, _)| *file == TRANSPORT)
        .map(|(_, source)| *source)
        .expect("清单里必须有 transport.slint");
    assert!(
        transport.contains(LABEL),
        "AI 抽屉按钮的 label⛔ 与键表同源（`src/input.rs` 第 3 步的 `(Character('a'), false, \
         true)` 那一臂 ⇒ `Action::ToggleAiDrawer`）"
    );
    let promises: Vec<Chord> = literals(transport)
        .iter()
        .flat_map(|literal| chords_in_literal(TRANSPORT, literal.line, &literal.text).chords)
        .collect();
    assert_eq!(
        promises
            .iter()
            .filter(|chord| chord.key == LogicalKey::Character('a'))
            .count(),
        1,
        "transport 里向用户承诺「呼出 AI 抽屉」的键位不是恰好一处：{promises:?}"
    );

    let name = registry_name(BUTTON_ID)
        .unwrap_or_else(|| panic!("`{REGISTRY_FILE}` 的登记表里必须登记着 {BUTTON_ID:?} 那一格"));
    assert!(
        chords_in_literal(REGISTRY_FILE, 0, &name).chords.is_empty(),
        "`{REGISTRY_FILE}` 给 {BUTTON_ID:?} 的登记名开始携带和弦：{name:?} ⇒ 那它就是第二处承诺，\
         必须与 `.slint` 的 label 同源并进 ① 的清单"
    );
}

/// ⑤（两源不漂移）：登记名里带和弦的那几格，必须与某条 `.slint` 字面**逐字相同**。
#[test]
fn registry_names_cannot_diverge_from_the_markup() {
    let slint = slint_literals();
    for chord in scan_corpus().chords {
        if chord.file != REGISTRY_FILE {
            continue;
        }
        assert!(
            slint.contains(&chord.literal),
            "{REGISTRY_FILE} 的登记名 {:?} 携带 `{}`，但 `.slint` 里**没有**逐字相同的字面 ⇒ \
             界面与控件树各说一把键（`D-UI-26` 那一类缺陷的第二个源头）",
            chord.literal,
            chord.printed
        );
    }
}

/// ⑥（扫描面完整）：磁盘上真实存在的 `.slint` 必须都在编译期清单里。
#[test]
fn every_slint_file_on_disk_is_in_the_embedded_inventory() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut on_disk = BTreeSet::new();
    let mut stack = vec![manifest_dir.join("ui")];
    while let Some(dir) = stack.pop() {
        for entry in
            std::fs::read_dir(&dir).unwrap_or_else(|err| panic!("读 {} 失败: {err}", dir.display()))
        {
            let path = entry.expect("目录项").path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if path.extension().is_some_and(|ext| ext == "slint") {
                let relative = path
                    .strip_prefix(manifest_dir)
                    .expect("清单里的路径都相对 crate 根")
                    .to_string_lossy()
                    .replace('\\', "/");
                on_disk.insert(relative);
            }
        }
    }
    let embedded: BTreeSet<&str> = SLINT_SOURCES.iter().map(|(file, _)| *file).collect();
    assert_eq!(
        on_disk.len(),
        SLINT_SOURCES.len(),
        "磁盘上有 {} 个 `.slint`，编译期清单只有 {} 项 ⇒ 有文件没被扫到（新增的要加进 \
         SLINT_SOURCES，`read_dir` 在本文件只用于这一条对账）",
        on_disk.len(),
        SLINT_SOURCES.len()
    );
    let missing: Vec<&String> = on_disk
        .iter()
        .filter(|name| !embedded.contains(&name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "这些 `.slint` 在磁盘上但⛔ 在清单里: {missing:?}"
    );
    // 承诺源 B 只有一个文件，且它⛔ 在 `.slint` 清单里 ⇒ 单独确认它被嵌进来了。
    assert!(
        !REGISTRY_SOURCE.is_empty(),
        "`{REGISTRY_FILE}` 必须编译期嵌入（④/⑤ 都靠它）"
    );
}

/// ⑦（承诺写全）：字面里出现修饰词就必须写成完整的 `修饰词 + 键名`；键名必须在词表里。
#[test]
fn every_key_hint_is_a_complete_promise() {
    let evidence = scan_corpus();
    assert!(
        evidence.incomplete.is_empty(),
        "这些字面提到了修饰词却没接上 `+ 键名`（承诺写坏了，或本守卫的词表需要扩）：{:?}",
        evidence.incomplete
    );
    assert!(
        evidence.unknown_keys.is_empty(),
        "这些和弦里的键名不在 `KEY_WORDS` 词表里：{:?} ⇒ 要么把它加进词表，要么这句文案⛔ \
         不该印出来（口径：⛔ 让认不出的键名蒙过去，那等于静默漏网）",
        evidence.unknown_keys
    );
}

/// ⑧（上下文口径）：文本框聚焦时 DAW 热键整体放行 —— 守卫数的是**画布**口径。
#[test]
fn the_promise_is_scored_against_the_canvas_context_only() {
    // `resolve_logical` 的第 2 步（文本域策略）会把**所有**非合成按键放行，
    // 因此"印在按钮上的键在输入框里没反应"⛔ 是本票的缺陷（那是 `ADR-0067` 未决的
    // 规范冲突）。本条把这条口径钉住：⛔ 让 ② 的绿被误解成"任何焦点下都生效"。
    let mut context = InputContext::new();
    context.set_focus(Focus::TextInput);
    for chord in scan_corpus().chords {
        let modifiers = Modifiers {
            ctrl: chord.command,
            shift: chord.shift,
            alt: chord.alt,
            meta: false,
        };
        assert_eq!(
            context.resolve_logical(chord.key, modifiers),
            Resolution::PassThrough,
            "`{}` 在文本域里竟然被 DAW 吞掉了 —— 这与 `ADR-0067` 登记的现状不符",
            chord.printed
        );
    }
}
