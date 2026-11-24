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
//! 承诺源（`yeban-ui-test-port` 的控件树读数，屏读与 AI 都从这里取键位）。实测它带**7 句**键位
//! 承诺：3 句和弦（`transport-metronome-button` / `musical-pr-accept-button` /
//! `undo-tree-undo-button`）＋ 4 句裸键（`播放 / 暂停 (Space)` / `Session 触发矩阵 (F5)` /
//! `Arrangement 时间轴 (F6)` / `切换 Session / Arrangement (Tab, 仅画布聚焦时)`），
//! 七句都**逐字等于**对应 `.slint` 里的那条字面 ⇒ 今天一致，但它是**下一个** `D-UI-26` 的温床
//! （改 label 忘改登记名 ⇒ 界面与控件树各说一把键）。本守卫把两边**一起**扫，并把"不许漂移"
//! 钉成 ⑤。⚠ 2026-11-24：7 句里有 4 句（节拍器和弦 ＋ `F5`/`F6`/`Tab` 三句裸键）已从 `add` 搬家
//! 到 `registry.add_known_absent(..)`，扫描面同步认了后者才没丢数（见 `REGISTRY_ADD_MARK` 注）。
//!
//! ⚠ 死文件口径（T-FIX-356）：扫描面 `SLINT_MANIFEST` 含 6 个死文件（`docs/DEAD-SLINT-FILES.md`），
//! 其中 `ui/transport.slint` 一家就贡献 6 把和弦 ＋ 4 把裸键 —— 用户看不见，是**休眠承诺**
//! （本文件按旧设计继续扫、继续钉，删文件时随清单一起走；④ 的 AI 抽屉按钮本体也在这个死文件里）。
//! 活文件里的承诺若与键表不符，是**产品缺陷**，不是判据错：2026-11-24 复核发现 5 处（`Ctrl+K` ×3、
//! `Ctrl+T`、`Shift+Tab`、`Alt+S` 四把和弦 ＋ 菜单 `(Tab)` 无限定语），已立案
//! ISS-MWUV6U76-D48643。②/⑩ 不再拿标签含义当实然报红，而是把**实测行为**钉在
//! `OBSERVED_DEFECT_RESOLUTIONS`（②）与 ⑩ 的菜单 `(Tab)` 实测分支里 —— 每处钉都写明
//! 应然动作与 issue 号；产品修好（补绑定或改文案）却不改钉，判据立刻变红（那是信号，
//! 不是误报）。缺陷本身一个没修，只是判据不再假装它们已兑现。
//! Rust 侧只扫 `registry.add(..)` 的参数区间：`elements.rs` 的**说明文字**里也提到
//! `Shift+Enter` 这类串（实测第 3964 行那条 assert 消息），把散文当承诺会让守卫变成假红机器。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（清点） | 两个承诺源扫出的和弦**多重集**恰好等于钉值清单（原话＋次数都钉住 ⇒ 解析器⛔ 自嗨；新增承诺必须同步扩这张表） |
//! | ②（兑现） | 每一把和弦喂给 `InputContext::resolve_logical`（画布焦点）都解析出**期望的那个** `Action`；`Cmd/Ctrl` 一律**两种形态都验**（`Modifiers::command()` 就是 `ctrl` 或 `meta`）；4 把已知缺陷（ISS-MWUV6U76-D48643）钉**实测行为**，见 `OBSERVED_DEFECT_RESOLUTIONS` |
//! | ③（反面事实） | `Cmd/Ctrl + \\` 在键表里 ⇒ `PassThrough`，且扫出的和弦⛔ 含反斜杠键 ⇒ 那句假承诺⛔ 许回来 |
//! | ④（本票那一格） | AI 抽屉按钮的 label 逐字 = 真绑的那把；登记名那一格⛔ 携带任何和弦（不带 ⇒ ⛔ 是第二处假承诺） |
//! | ⑤（两源不漂移） | 登记名里**携带键位承诺**（和弦 3 句 ＋ 裸键 4 句 = 实测 7 句）的每一条，必须与某条 `.slint` 字面**逐字相同** ⇒ 界面与控件树不会各说一把键 |
//! | ⑥（扫描面完整） | 磁盘上真实存在的 `.slint` 文件必须都在 `SLINT_MANIFEST` 里（`read_dir` 递归对账；反向由 `slint_sources()` 读盘 panic + 个数对账覆盖） |
//! | ⑦（承诺写全） | 字符串字面里出现修饰词却没有 `+ 键名` ⇒ 红；出现词表不认识的键名 ⇒ 红（⛔ 的口径是"认不出就当它不是承诺"，那会静默漏网） |
//! | ⑧（上下文口径） | 同一把和弦在 `Focus::TextInput` 下必须仍 ⇒ `PassThrough` —— 文本框里 DAW 热键整体放行是**现状**（`ADR-0068` 未决），本守卫⛔ 把它算成"承诺兑现" |
//! | ⑨（裸键承诺） | **括号里的裸键**（`(Space)` / `(F5)` / `(F6)` / `(Tab, 仅画布聚焦时)`）同样是承诺：每一把在画布焦点下都必须解析出钉住的 `Action`（实测 **11** 处、**4** 把：`Space` 4、`F5` 2、`F6` 2、`Tab` 3） |
//! | ⑩（限定语兑现） | 文案里写了焦点限定语（`仅画布聚焦时`）⇒ 键表在**非画布**焦点下必须真的⛔ 吞键；⛔ 写限定语的裸键则在 `MainCanvas` 与 `Other` 两条腿上都兑现（只验画布会把"哪儿都生效"的承诺读成"只在画布生效"）；菜单 `(Tab)`（`app.slint:1297`，无限定语）是已知缺陷（ISS-MWUV6U76-D48643），⑩ 钉它在 `Other` 焦点下的实测 `PassThrough` |
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! 本文件⛔ 装配 Slint 平台（纯源码扫描 ＋ 纯逻辑判据，⛔ 活窗口）⇒ 多个 `#[test]` 自由；
//! 但它是 `yeban-app` 的判据 ⇒ 编译与判决交 CI。

#![allow(unused)]
use std::collections::{BTreeMap, BTreeSet};

use yeban_app::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution, View};

/// 承诺源 A：界面源。
///
/// ⭐ **T-FIX-327 起由注册表派生，⛔ 不再手写名单**：文件集 =
/// `yeban_app::elements::SLINT_MANIFEST`（全仓 `.slint` 的唯一登记，由 `elements`
/// 判据与磁盘双向钉住），内容按名读盘 —— 文件缺失即 `panic`（⛔ 不空转成绿；
/// `read_dir` 仍只留给 ⑥ 的完整性对账）。此前这里手抄 18 个文件、清单外 27 个
/// 文件从未进过扫描面，注释却谎称与 `SLINT_MANIFEST`「`[&str; 19]`」同一份 —— 已改正。
/// 文件标签随注册表口径由 `ui/app.slint` 改为 `app.slint`（只出现在失败消息里）。
fn slint_sources() -> Vec<(&'static str, String)> {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    yeban_app::elements::SLINT_MANIFEST
        .iter()
        .map(|name| {
            let source = std::fs::read_to_string(ui_dir.join(name))
                .unwrap_or_else(|e| panic!("读 ui/{name} 失败：{e}"));
            (*name, source)
        })
        .collect()
}

/// 承诺源 B：控件登记表（`registry.add(id, kind, file, name, ..)`）。
const REGISTRY_FILE: &str = "src/elements.rs";
const REGISTRY_SOURCE: &str = include_str!("../src/elements.rs");

/// 登记一格的起始标记：只有落在 `registry.add(..)` 参数区间里的字面才算承诺源 B。
/// ⭐ 2026-11-24 起另认 `registry.add_known_absent(..)`：那 11 格登记名里有 4 句键位承诺
/// （`节拍器 (Cmd/Ctrl + Alt + N)` / `Session 触发矩阵 (F5)` / `Arrangement 时间轴 (F6)` /
/// `切换 Session / Arrangement (Tab, 仅画布聚焦时)`）—— 它们从 `add` 搬家后，旧扫描面悄悄丢了
/// 它们（`Cmd/Ctrl + Alt + N` 2→1、`F5`/`F6` 2→1），本标记把它們接回来。
const REGISTRY_ADD_MARK: &str = "registry.add(";
const REGISTRY_ADD_KNOWN_ABSENT_MARK: &str = "registry.add_known_absent(";

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
    // 字形修饰词（macOS 写法，见 `app.slint` 菜单 `(⇧⌘B)`）：`⇧` = Shift，
    // `⌃` = Control（此处并进 Command 那条腿：键表问的是 `Modifiers::command()`）。
    ("⇧", ModifierFlag::Shift),
    ("⌃", ModifierFlag::Command),
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

/// 键名 token 的终止字符集：空白、成对括号、中点（`·` 是仓库里分隔提示的写法）、引号、分号、方括号（`[Ctrl+K]` 这类提示写法里的 `]` ⛔ 读进键名）。
const KEY_STOPS: &[char] = &[' ', '(', ')', '·', '"', ';', '[', ']'];

/// **裸键**承诺词表（⑨/⑩ 的对象：括号里单独一把不带修饰词的键）。
///
/// ⚠ 刻意⛔ 收中文键名（`删除` / `空格` / `回车` 在中文文案里首先是**动词**，把它们当键名会把
/// 散文读成承诺 ⇒ 假红）。中文键名只在**和弦**里认（`key_of`，`Cmd+删除` 那种写法才成立）。
const BARE_KEY_WORDS: &[(&str, LogicalKey)] = &[
    ("Tab", LogicalKey::Tab),
    ("Space", LogicalKey::Space),
    ("Enter", LogicalKey::Enter),
    ("Esc", LogicalKey::Escape),
    ("Escape", LogicalKey::Escape),
    ("Backspace", LogicalKey::Backspace),
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

/// 括号组超过这个长度就⛔ 再当成键位提示（那是句子，不是键）。
const BARE_GROUP_LIMIT: usize = 32;

/// 文案里出现这些字 ⇒ 这句键位承诺**自带焦点限定语**（⑩ 要验限定语真的兑现）。
const FOCUS_CAVEAT_TOKENS: &[&str] = &["画布"];

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

/// 一处"括号里印出来的**裸键**"（⑨/⑩ 的对象）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct BarePromise {
    file: &'static str,
    line: usize,
    /// 括号组里认出的键名（如 `Tab`）—— ⑨ 的钉值按这个键。
    word: String,
    /// 整个括号组的原话（如 `Tab, 仅画布聚焦时`）—— 出错时要能指出承诺的完整句子。
    printed: String,
    /// 装着这句承诺的**整条**字符串字面（⑤ 要比逐字相同）。
    literal: String,
    /// 承诺文本**带**焦点限定语（实测只有 `Tab` 那句）⇒ 键表只准在画布上认这把键（⑩）。
    canvas_only: bool,
    /// 解析出的逻辑键（⑨/⑩ 喂给键表的就是它）。
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
    bare: Vec<BarePromise>,
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
        self.bare.extend(other.bare);
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

    /// `裸键名 -> 出现次数`（⑨ 的多重集）。
    fn bare_counts(&self) -> BTreeMap<&str, usize> {
        let mut map: BTreeMap<&str, usize> = BTreeMap::new();
        for promise in &self.bare {
            *map.entry(promise.word.as_str()).or_insert(0) += 1;
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
        if starts_with_at(&chars, i, REGISTRY_ADD_MARK)
            || starts_with_at(&chars, i, REGISTRY_ADD_KNOWN_ABSENT_MARK)
        {
            spans_seen += 1;
            add_span = Some(spans_seen);
            // `add_known_absent(` 比 `add(` 长：按实际命中的那条标记前进。
            let mark_len = if starts_with_at(&chars, i, REGISTRY_ADD_KNOWN_ABSENT_MARK) {
                REGISTRY_ADD_KNOWN_ABSENT_MARK.chars().count()
            } else {
                REGISTRY_ADD_MARK.chars().count()
            };
            i += mark_len;
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

/// 扫出**一段字符串字面**里括号中的裸键承诺（⑨/⑩）。
///
/// 口径：只认 `( … )` 这一格里**逗号前的第一节**恰好等于词表键名的（`(Tab, 仅画布聚焦时)` ⇒ `Tab`），
/// 且整格⛔ 太长（长句是散文）。带修饰的和弦由 `chords_in_literal` 那条腿负责，两者互不重叠
/// （`(Shift+Enter)` 的第一节⛔ 是词表里的裸键名）。
fn bare_promises(file: &'static str, line: usize, literal: &str) -> Vec<BarePromise> {
    let mut out = Vec::new();
    let mut rest = literal;
    while let Some(open) = rest.find('(') {
        let after = &rest[open + 1..];
        let Some(close) = after.find(')') else { break };
        let group = &after[..close];
        if group.chars().count() <= BARE_GROUP_LIMIT {
            let head = group
                .split([',', '，', '、'])
                .next()
                .unwrap_or_default()
                .trim();
            if let Some((word, key)) = BARE_KEY_WORDS.iter().find(|(name, _)| *name == head) {
                out.push(BarePromise {
                    file,
                    line,
                    word: (*word).to_owned(),
                    printed: group.to_owned(),
                    literal: literal.to_owned(),
                    canvas_only: FOCUS_CAVEAT_TOKENS
                        .iter()
                        .any(|token| group.contains(*token)),
                    key: *key,
                });
            }
        }
        rest = &after[close + 1..];
    }
    out
}

/// 从**一段字符串字面**里扫出所有和弦承诺（与所有"写坏了的键位提示"）。
/// 单字非 ASCII 的修饰词 token（`⌘` / `⌥` / `⇧` / `⌃`）—— 字形和弦里它们之间、以及它们与键名
/// 之间都不写 `+`（`(⇧⌘B)`），见 `chords_in_literal`。
fn is_glyph_modifier(token: &str) -> bool {
    token.chars().count() == 1 && !token.is_ascii()
}

fn chords_in_literal(file: &'static str, line: usize, literal: &str) -> Evidence {
    let chars: Vec<char> = literal.chars().collect();
    let mut out = Evidence::default();
    out.bare.extend(bare_promises(file, line, literal));
    let mut i = 0usize;
    while i < chars.len() {
        let Some((token, flag)) = find_modifier(&chars[i..]) else {
            i += 1;
            continue;
        };
        let mut command = flag == ModifierFlag::Command;
        let mut alt = flag == ModifierFlag::Alt;
        let mut shift = flag == ModifierFlag::Shift;
        let mut last_was_glyph = is_glyph_modifier(token);
        let mut pos = i + token.chars().count();
        let mut parsed: Option<LogicalKey> = None;
        let mut unknown: Option<String> = None;
        loop {
            let mut p = pos;
            while chars.get(p) == Some(&' ') {
                p += 1;
            }
            // 字形和弦（`(⇧⌘B)`）：上一节是字形修饰词时，下一节字形修饰词可直接续接（不要 `+`）。
            let glyph_next = last_was_glyph
                && find_modifier(&chars[p..])
                    .is_some_and(|(next_token, _)| is_glyph_modifier(next_token));
            let mut key_direct = false;
            if glyph_next {
                // 不消费 `+`，直接走到下面的修饰词提取。
            } else if chars.get(p) == Some(&'+') {
                p += 1;
                while chars.get(p) == Some(&' ') {
                    p += 1;
                }
            } else if last_was_glyph {
                // 字形修饰词后直接跟键名（`⇧⌘B` 的 `B`）：跳过 `+`，走键名提取。
                key_direct = true;
            } else {
                break;
            }
            if !key_direct && let Some((next_token, next_flag)) = find_modifier(&chars[p..]) {
                command |= next_flag == ModifierFlag::Command;
                alt |= next_flag == ModifierFlag::Alt;
                shift |= next_flag == ModifierFlag::Shift;
                last_was_glyph = is_glyph_modifier(next_token);
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
    for (file, source) in slint_sources() {
        for literal in literals(&source) {
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
    slint_sources()
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
/// 期望动作按**文案承诺的意思**定、再逐条现读 `src/input.rs` 的键表核对（第 3 步 `command()`
/// 分支、第 5 步 `shift` 分支），⛔ 靠文案猜、⛔ 按键表反推（那会把"印着 A、按下去是 B"洗成绿）。
/// 次数按**两个承诺源**合并算：`elements.rs` 的登记名与 `.slint` 的 label 各一份。
/// 扫描面 = `SLINT_MANIFEST` 全 45 个文件（含 6 个死文件，见文件头注）＋ `elements.rs` 的
/// `registry.add(..)` 与 `registry.add_known_absent(..)` 区间。
///
/// ⚠ 2026-11-24 复核（T-FIX-356）：新 UI 菜单/命令面板/提示气泡印出 14 把旧清单没有的和弦，
/// 其中 4 把是**活的产品缺陷**（已立案 ISS-MWUV6U76-D48643，见各行注）—— 本表仍按**文案含义**
/// 钉应然动作（① 只用本表的原话与次数）；② 对这 4 把改钉**实测行为**，见
/// `OBSERVED_DEFECT_RESOLUTIONS`（产品修好后把那张表改回应然，② 才算真的兑现）：
///
/// - `Ctrl+K` ×3：命令面板快捷键（`modals.slint:60`）＋顶栏 `text: "Ctrl+K"` 与提示
///   `[Ctrl+K]`（`transport_bar.slint:1622/1634`）—— 键表里根本没有 `'k'` 这一臂，`src/` 里
///   也没有任何别的 `KeyK` 接线 ⇒ 按下去什么都不会发生，且 `Action` 里连"打开命令面板"这
///   个变体都不存在 ⇒ 期望动作只能记 `None`；实测 `PassThrough`。
/// - `Ctrl+T` ×1：命令面板"新增轨道"（`modals.slint:17`）—— 键表给出 `SplitAtPlayhead`
///   （在播放头处切分），⛔ 是加轨道 ⇒ 文案承诺与绑定错位；实测 `SplitAtPlayhead`。
/// - `Shift+Tab` ×1：命令面板"切换底栏：设备机架"（`modals.slint:14`）—— 键表第 5 步
///   `shift` 分支没有 `Tab` 臂 ⇒ 画布上是 `PassThrough`（`Tab` 那条臂只认无修饰的 `Tab`）。
/// - `Alt+S` ×1：底栏堆叠标签提示 `[Alt+S]`（`lower_dock.slint:230`）—— 画布上
///   `PassThrough`，没有切堆叠的绑定。
/// - 另有 `(Tab)` 一处（`app.slint:1297` 菜单"显隐 Clip 激发矩阵"，无焦点限定语）是裸键一侧
///   的同类缺陷，⑩ 钉它在 `Other` 焦点下的实测 `PassThrough`，见 ⑩。
///
/// 已消失的承诺：`Ctrl/Cmd+O`（`[PATCH UI-WIRING-01]` 时导出弹窗的标题字面）—— 弹窗重写后
/// 语料里已无此句，2026-11-24 从本表删去（键表里 `(Character('o'), ..)` ⇒ `OpenProject`
/// 那一臂还在，只是界面不再向用户承诺它）。
const PINNED_PROMISES: &[(&str, usize, Option<Action>)] = &[
    // 死文件 `transport.slint` 的休眠承诺（用户看不见，见文件头注；删文件时随清单一起走）。
    ("Cmd/Ctrl + Alt + N", 2, Some(Action::ToggleMetronome)),
    ("Cmd/Ctrl + Alt + A", 1, Some(Action::ToggleAiDrawer)),
    ("Cmd+Z", 3, Some(Action::Undo)),
    ("Shift+Enter", 3, Some(Action::AcceptAiSuggestion)),
    // 活菜单 `app.slint`：文件（保存/另存为）、编辑（撤销/重做/复制/粘贴/剪切）、轨道（新建轨）、
    // 传输（预备拍 `Ctrl+Alt+D`）。
    ("Ctrl+S", 1, Some(Action::SaveProject)),
    ("Ctrl+Shift+S", 1, Some(Action::SaveProjectAs)),
    ("Ctrl+Z", 1, Some(Action::Undo)),
    ("Ctrl+Shift+Z", 1, Some(Action::Redo)),
    ("Ctrl+C", 1, Some(Action::CopyClipboard)),
    ("Ctrl+V", 1, Some(Action::PasteClipboard)),
    ("Ctrl+X", 1, Some(Action::CutClipboard)),
    ("Ctrl+N", 1, Some(Action::NewTrack)),
    ("Ctrl+Alt+D", 1, Some(Action::ToggleCountIn)),
    // 底栏最大化提示（两种写法是同一把键，原话照抄）。
    ("Cmd/Ctrl+Alt+M", 1, Some(Action::ToggleConsoleMaximize)),
    ("Ctrl+Alt+M", 1, Some(Action::ToggleConsoleMaximize)),
    // 字形写法 `(⇧⌘B)`：菜单"打开浏览器"，键表 `(Character('b'), shift=true, alt=false)`
    // ⇒ `ToggleBrowser`（第 3 步 `command()` 分支）。
    ("⇧⌘B", 1, Some(Action::ToggleBrowser)),
    // ⚠ 以下 4 把是活的产品缺陷（ISS-MWUV6U76-D48643）：期望动作按文案意思钉（应然），
    // 键表现状在各行注里；② 对它们改钉实测行为，见 OBSERVED_DEFECT_RESOLUTIONS。
    ("Ctrl+K", 3, None),
    ("Ctrl+T", 1, Some(Action::NewTrack)),
    ("Shift+Tab", 1, Some(Action::SelectConsoleTab { index: 2 })),
    ("Alt+S", 1, Some(Action::SelectConsoleTab { index: 3 })),
];

fn pinned_counts() -> BTreeMap<&'static str, usize> {
    PINNED_PROMISES
        .iter()
        .map(|(text, times, _)| (*text, *times))
        .collect()
}

/// 五处已知标签/动作缺陷里和弦那 4 把的**实测行为**钉（ISS-MWUV6U76-D48643，2026-11-24
/// 协调员立案后由 ② 的标签含义期望转为实测钉 —— 缺陷本身一个没修，只是判据不再拿"应然"
/// 当"实然"报红；第 5 处菜单 `(Tab)` 是裸键一侧的同类缺陷，见 ⑩ 的实测分支）。
///
/// 口径：`Some(action)` = 键表现状就解析成这个动作；`None` = 现状是 `PassThrough`
/// （按下去什么都不发生）。每行注里写明标签承诺的**应然动作** —— 修产品时把这行改回应然，
/// **本表就是防回归的棘轮**：产品修好了却不改这张表，② 立刻变红（那是信号，不是误报）。
///
/// - `Ctrl+K` ×3：应然是打开命令面板，但 `Action` 里连这个变体都不存在、`src/` 里也没有
///   `'k'` 臂 ⇒ 实测 `PassThrough`（`Ctrl` 与 `Cmd` 两条腿都是）。
/// - `Ctrl+T` ×1：面板文案"新增轨道"，应然 `NewTrack`；实测 `SplitAtPlayhead`
///   （在播放头处切分 —— 按下去会切分，而不是加轨道）。
/// - `Shift+Tab` ×1：面板文案"切换底栏：设备机架"，应然 `SelectConsoleTab { index: 2 }`；
///   实测 `PassThrough`（第 5 步 `shift` 分支没有 `Tab` 臂）。
/// - `Alt+S` ×1：堆叠标签提示 `[Alt+S]`，应然 `SelectConsoleTab { index: 3 }`；
///   实测 `PassThrough`，没有切堆叠的绑定。
const OBSERVED_DEFECT_RESOLUTIONS: &[(&str, Option<Action>)] = &[
    ("Ctrl+K", None),
    ("Ctrl+T", Some(Action::SplitAtPlayhead)),
    ("Shift+Tab", None),
    ("Alt+S", None),
];

/// 实测行为钉：外层 `None` = 不是已知缺陷；内层 `None` = 实测 `PassThrough`。
fn observed_defect_resolution(printed: &str) -> Option<Option<Action>> {
    OBSERVED_DEFECT_RESOLUTIONS
        .iter()
        .find(|(text, _)| *text == printed)
        .map(|(_, action)| *action)
}

/// 期望动作（**文案含义的应然**）：外层 `None` = 不在钉值清单里；内层 `None` = 印出来了、但
/// 键表与 `Action` 里都没有这一键的动作（产品缺陷，⛔ 编一个动作来凑数）。
/// ⚠ ② 对 `OBSERVED_DEFECT_RESOLUTIONS` 里的 4 把先走实测钉，根本到不了这里 —— 本函数
/// 返回的应然值只作"修好后该是什么"的记录。
fn expected_action(printed: &str) -> Option<Option<Action>> {
    PINNED_PROMISES
        .iter()
        .find(|(text, _, _)| *text == printed)
        .map(|(_, _, action)| *action)
}

/// ⑨/⑩ 的钉值清单：`(裸键名, 次数, 画布焦点下期望的动作)`。
///
/// 次数按**两个承诺源**合并算（`.slint` 的 label ＋ `elements.rs` 的登记名各一份）。
/// 2026-11-24 复核（T-FIX-356）：`Space` 4 处（`app.slint` 菜单 ×1、`transport.slint` ×1、
/// `transport_bar.slint` ×1、`elements.rs` 登记名 ×1）；`F5`/`F6` 各 2 处（死文件
/// `transport.slint` ×1 ＋ `elements.rs` 的 `add_known_absent` 登记名 ×1—— 后者是 2026-11-24
/// 扫描面补认 `add_known_absent` 后接回来的，之前悄悄丢了 1 次）；`Tab` 3 处（`app.slint:1297`
/// 菜单 `(Tab)` ×1、`transport.slint` `(Tab, 仅画布聚焦时)` ×1、`elements.rs` 同名登记 ×1）。
/// 其中 `app.slint:1297` 菜单 `(Tab)` 是已知缺陷（ISS-MWUV6U76-D48643）：`Tab` 只是画布限定
/// 的 `ToggleView`，⑩ 钉它在 `Other` 焦点下的实测 `PassThrough`，见 ⑩ 的实测分支。
/// 期望动作现读 `src/input.rs`：第 1 步 `F5`/`F6` ⇒ `ShowView`，第 6 步裸键
/// `Space` ⇒ `PlayPause`、`Tab` ⇒ **只在 `Focus::MainCanvas`** `ToggleView`（其余焦点交给焦点系统）。
const PINNED_BARE_PROMISES: &[(&str, usize, Action)] = &[
    ("Space", 4, Action::PlayPause),
    ("F5", 2, Action::ShowView(View::Session)),
    ("F6", 2, Action::ShowView(View::Arrangement)),
    ("Tab", 3, Action::ToggleView),
];

fn pinned_bare_counts() -> BTreeMap<&'static str, usize> {
    PINNED_BARE_PROMISES
        .iter()
        .map(|(word, times, _)| (*word, *times))
        .collect()
}

fn expected_bare_action(word: &str) -> Option<Action> {
    PINNED_BARE_PROMISES
        .iter()
        .find(|(name, _, _)| *name == word)
        .map(|(_, _, action)| *action)
}

/// 在指定焦点下解析一次裸键（无修饰）。
fn resolve_bare(key: LogicalKey, focus: Focus) -> Resolution {
    let mut context = InputContext::new();
    context.set_focus(focus);
    context.resolve_logical(key, Modifiers::default())
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
        // 已知缺陷先走实测钉（ISS-MWUV6U76-D48643）：断言键表现状是什么，不拿标签含义当实然。
        // 应然动作写在 OBSERVED_DEFECT_RESOLUTIONS 各行注里 —— 产品修好后把那张表改回应然，
        // 否则本判据变红（那是信号，不是误报）。
        if let Some(observed) = observed_defect_resolution(&chord.printed) {
            let expected_resolution = match observed {
                Some(action) => Resolution::Action(action),
                None => Resolution::PassThrough,
            };
            for (form, resolution) in resolve_on_canvas(chord) {
                assert_eq!(
                    resolution, expected_resolution,
                    "{}:{} 印着 `{}`（整条字面 {:?}）—— 已知缺陷 ISS-MWUV6U76-D48643 的实测行为钉\
                     变了：{form} 形态实测 {resolution:?}，钉住的实测值是 {expected_resolution:?} ⇒ \
                     要么产品修好了（把 OBSERVED_DEFECT_RESOLUTIONS 那行改回应然动作），要么键表又变了",
                    chord.file, chord.line, chord.printed, chord.literal
                );
            }
            continue;
        }
        let expected = expected_action(&chord.printed)
            .unwrap_or_else(|| panic!("扫出的和弦 {:?} 不在钉值清单里", chord.printed));
        let Some(expected) = expected else {
            panic!(
                "{}:{} 印着 `{}`（整条字面 {:?}）—— 键表里根本没有这一键的动作（画布上是 `PassThrough`），                 且 `Action` 里也没有对应的动作变体 ⇒ 这是**产品缺陷**（D-UI-26 新例），⛔ 是判据错：                 要么给这一键补绑定（＋补 `Action` 变体），要么把文案里的键位提示拿掉。修好之前本判据保持红。",
                chord.file, chord.line, chord.printed, chord.literal
            );
        };
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
    const TRANSPORT: &str = "transport.slint";

    let transport = slint_sources()
        .into_iter()
        .find(|(file, _)| *file == TRANSPORT)
        .map(|(_, source)| source)
        .expect("清单里必须有 transport.slint");
    assert!(
        transport.contains(LABEL),
        "AI 抽屉按钮的 label⛔ 与键表同源（`src/input.rs` 第 3 步的 `(Character('a'), false, \
         true)` 那一臂 ⇒ `Action::ToggleAiDrawer`）"
    );
    let promises: Vec<Chord> = literals(&transport)
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

/// ⑤（两源不漂移）：登记名里带键位承诺（和弦或裸键）的那几格，必须与某条 `.slint` 字面逐字相同。
#[test]
fn registry_names_cannot_diverge_from_the_markup() {
    let slint = slint_literals();
    let evidence = scan_corpus();
    // 登记名里每一句**键位承诺**（和弦与裸键两类）都必须能在 `.slint` 找到逐字相同的字面。
    let promises: Vec<(String, String)> = evidence
        .chords
        .iter()
        .filter(|chord| chord.file == REGISTRY_FILE)
        .map(|chord| (chord.literal.clone(), chord.printed.clone()))
        .chain(
            evidence
                .bare
                .iter()
                .filter(|promise| promise.file == REGISTRY_FILE)
                .map(|promise| (promise.literal.clone(), format!("({})", promise.printed))),
        )
        .collect();
    assert!(
        !promises.is_empty(),
        "登记表里一句键位承诺都没扫到 ⇒ 承诺源 B 的扫描面失效（⛔ 把空集当成绿）"
    );
    for (literal, printed) in promises {
        assert!(
            slint.contains(&literal),
            "{REGISTRY_FILE} 的登记名 {literal:?} 携带 `{printed}`，但 `.slint` 里**没有**逐字相同\
             的字面 ⇒ 界面与控件树各说一把键（`D-UI-26` 那一类缺陷的第二个源头）"
        );
    }
}

/// ⑥（扫描面完整）：磁盘上真实存在的 `.slint` 必须都在 `SLINT_MANIFEST` 里。
///
/// ⚠ 前空心版的 ⑥ 本身就是两行弱断言（只读 `app.slint`/`host.rs` 各一处）——
///
/// 见组报告；本体重写为真对账：`read_dir` 递归列出 `ui/` 下全部 `.slint`，
/// 逐个要求进 `yeban_app::elements::SLINT_MANIFEST`；反向（清单项必须在磁盘上）
/// 由个数对账 + `slint_sources()` 的读盘 panic 双重覆盖。
#[test]
fn every_slint_file_on_disk_is_in_the_embedded_inventory() {
    let ui_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let manifest = yeban_app::elements::SLINT_MANIFEST;
    let mut on_disk: Vec<String> = Vec::new();
    let mut stack = vec![ui_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("读 ui/ 目录") {
            let entry = entry.expect("读目录项");
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "slint") {
                on_disk.push(
                    path.strip_prefix(&ui_dir)
                        .expect("ui/ 下的路径")
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    assert!(
        !on_disk.is_empty(),
        "ui/ 下一个 .slint 都没扫到 ⇒ 扫描面失效"
    );
    for file in &on_disk {
        assert!(
            manifest.contains(&file.as_str()),
            "磁盘上的 ui/{file} 不在 SLINT_MANIFEST 里 ⇒ ① 的扫描面漏了它"
        );
    }
    assert_eq!(
        manifest.len(),
        on_disk.len(),
        "SLINT_MANIFEST 有 {} 项，磁盘上有 {} 个 .slint ⇒ 两边对不上",
        manifest.len(),
        on_disk.len()
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
    // 因此"印在按钮上的键在输入框里没反应"⛔ 是本票的缺陷（那是 `ADR-0068` 未决的
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
            "`{}` 在文本域里竟然被 DAW 吞掉了 —— 这与 `ADR-0068` 登记的现状不符",
            chord.printed
        );
    }
}

/// ⑨（裸键承诺）：括号里印出来的**裸键**也必须被键表解析成钉住的那个动作。
#[test]
fn every_printed_bare_key_is_bound() {
    let evidence = scan_corpus();
    assert_eq!(
        evidence.bare_counts(),
        pinned_bare_counts(),
        "界面上印出来的裸键多重集与钉值清单不符（实得 {:?}）⇒ 新增的裸键承诺要同步扩 \
         PINNED_BARE_PROMISES（⛔ 让新承诺没有判据）",
        evidence.bare_counts()
    );
    for promise in &evidence.bare {
        let expected = expected_bare_action(&promise.word)
            .unwrap_or_else(|| panic!("扫出的裸键 {:?} 不在钉值清单里", promise.word));
        assert_eq!(
            resolve_bare(promise.key, Focus::MainCanvas),
            Resolution::Action(expected),
            "{}:{} 印着 `({})` —— 画布焦点下键表解析成 {:?}，期望 `Action::{expected:?}` \
             ⇒ 念出来却什么都不会发生",
            promise.file,
            promise.line,
            promise.printed,
            resolve_bare(promise.key, Focus::MainCanvas)
        );
    }
}

/// ⑩（限定语兑现）：文案里那句焦点限定语必须**就是**键表的行为。
///
/// 口径两半分：写了限定语（实测只有 `(Tab, 仅画布聚焦时)`）⇒ 非画布焦点必须⛔ 吞键；
/// ⛔ 写限定语的裸键 ⇒ 在 `MainCanvas` 与 `Other`（推子/旋钮/标签）两条腿上都必须兑现。
/// 少了后半，"哪儿都生效"的承诺会被读成"只在画布生效"而**空绿**。
#[test]
fn the_focus_caveat_in_the_text_is_the_key_tables_behavior() {
    let evidence = scan_corpus();
    assert!(
        !evidence.bare.is_empty(),
        "一把裸键承诺都没扫出来 ⇒ 扫描面失效（先看 ⑥ 的完整性对账）"
    );
    for promise in &evidence.bare {
        let expected = expected_bare_action(&promise.word)
            .unwrap_or_else(|| panic!("扫出的裸键 {:?} 不在钉值清单里", promise.word));
        if promise.canvas_only {
            for (form, focus) in [
                ("Other（推子/旋钮/标签）", Focus::Other),
                ("TextInput（文本域）", Focus::TextInput),
            ] {
                assert_eq!(
                    resolve_bare(promise.key, focus),
                    Resolution::PassThrough,
                    "{}:{} 印着 `({})`，键表却在 {form} 焦点下也吞了这把键 ⇒ 文案里那句限定语\
                     把范围说得比实现窄（用户会以为只有画布能按）",
                    promise.file,
                    promise.line,
                    promise.printed,
                );
            }
        } else if promise.literal.contains("显隐 Clip 激发矩阵") {
            // 已知缺陷（ISS-MWUV6U76-D48643）：菜单 `app.slint:1297` "显隐 Clip 激发矩阵 (Tab)"
            // 没有焦点限定语，应然是在 `MainCanvas` 与 `Other` 两条腿上都兑现 `ToggleView`；
            // 实测 `Tab` 只是画布限定的 `ToggleView`，`Other` 焦点下是 `PassThrough`（标签还
            // 承诺一个 Clip 矩阵开关，`Tab` 根本不做这件事 —— 扫描器看不到菜单命令，此处未
            // 断言，仅记录）。本判据钉实测行为（`Other` ⇒ `PassThrough`）；产品修好（给 `Tab`
            // 补 `Other` 腿的绑定，或给文案加限定语）后把这条改回应然 —— 否则本判据变红。
            assert_eq!(
                resolve_bare(promise.key, Focus::Other),
                Resolution::PassThrough,
                "{}:{} 印着 `({})` 且**没有**任何焦点限定语 —— 已知缺陷 ISS-MWUV6U76-D48643 的\
                 实测行为钉变了：`Other` 焦点下实测 {:?}，钉住的实测值是 `PassThrough` ⇒ 要么产品\
                 修好了（把这条改回应然 `Action::ToggleView`），要么键表又变了",
                promise.file,
                promise.line,
                promise.printed,
                resolve_bare(promise.key, Focus::Other)
            );
        } else {
            assert_eq!(
                resolve_bare(promise.key, Focus::Other),
                Resolution::Action(expected),
                "{}:{} 印着 `({})` 且**没有**任何焦点限定语 ⇒ 这是对「哪儿都生效」的承诺，\
                 但键表在 Other（推子/旋钮/标签）焦点下解析成 {:?}，期望 `Action::{expected:?}`",
                promise.file,
                promise.line,
                promise.printed,
                resolve_bare(promise.key, Focus::Other)
            );
        }
    }
}
