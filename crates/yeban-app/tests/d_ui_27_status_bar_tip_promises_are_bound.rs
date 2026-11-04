//! 状态栏那句**快捷键提示**印出来的每一把键，必须是键表里真绑的那一把（缺口 `D-UI-27`）。
//!
//! ## 本文件要证的句子
//!
//! `ui/status_bar.slint` 的 `tip` 是**画在屏幕上的**一句话（`text: root.tip;`），
//! 而 `app.slint` 的 `StatusBar { … }` 实例**没有**传 `tip`、宿主也**没有** `set_tip(` 写者
//! ⇒ 用户在窗口底部看到的逐字就是 `.slint` 里那个默认字面量。它是对用户的**承诺**：
//!
//! ```text
//! 空格 播放/暂停 · Tab 切视图 (画布聚焦时) · 1–5 工具 · [ ] A/B 盲听
//! ```
//!
//! ## 为什么 `D-UI-26` 的守卫⛔ 覆盖不到这一句
//!
//! `tests/d_ui_26_labelled_chords_are_bound.rs` 扫的是两种形状：**带修饰的和弦**
//! （`Cmd/Ctrl + X`）与**括号里的裸键**（`(Space)` / `(F5)`）。本句里四段承诺**一段都不是**
//! 这两种形状：
//!
//! | 段 | 形状 | D-UI-26 的扫描面 |
//! | :--- | :--- | :--- |
//! | `空格 播放/暂停` | **中文键名**、无括号 | ⛔ 看不见 |
//! | `Tab 切视图 (画布聚焦时)` | 括号里装的是**限定语**⛔ 键名 | ⛔ 会把限定语当键名 |
//! | `1–5 工具` | **数字区间**（连接号是 en-dash） | ⛔ 看不见 |
//! | `[ ] A/B 盲听` | **方括号键**，两把写成一段 | ⛔ 看不见 |
//!
//! ⇒ 承诺与键表的一致性今天**没有任何机械防线**：改这一句话（比如顺手加一把 `Q 量化`）
//! 就能造出一个新的"念出来却什么都不会发生"，与 `D-UI-26` 那把 `Cmd/Ctrl+\` 同罪。
//!
//! ## 判据
//!
//! | 判据 | 证的事 |
//! | :--- | :--- |
//! | ①（清点） | 从字面量解析出的承诺**多重集**逐字等于钉死的 **9** 把键（键名＋它说的**原话**＋限定语都钉住）；段数 == **4**；词表不认识的段 == **0**（⇒ 解析器⛔ 自嗨，新增承诺必须同步扩这张表） |
//! | ②（兑现·画布） | 每一把喂给 `InputContext::resolve_logical`（`Focus::MainCanvas`）都解析出**期望的那个** `Action` |
//! | ③（兑现·Other ＋ 限定语双向对账） | ⛔ 写限定语的键在 `Focus::Other`（推子/旋钮/标签）下也必须兑现；写了 `(画布聚焦时)` 的键在 `Other` 下必须⛔ 吞键 —— 文案的限定语与键表的门**必须同进同退**（口径同 `D-UI-26` ⑩） |
//! | ④（文本域口径） | 九把键在 `Focus::TextInput` 下全部 ⇒ `PassThrough` —— 文本框里 DAW 热键整体放行是**现状**（`ADR-0068` 未决），本守卫⛔ 把它算成"承诺兑现" |
//! | ⑤（画的 == 扫的） | 三条一起钉：`status_bar.slint` 里 `text: root.tip;` 还在（字面量真的上屏）；`app.slint` 的 `StatusBar {` 实例⛔ 传 `tip:`；`src/host.rs` 的代码行里 `set_tip(` 命中 **0** 处 ⇒ 本文件扫的那一份⛔ 许是死字面量 |
//! | ⑥（一段⛔ 两把键） | 每一段里词表键名只许出现**一次**（`[ ]` 那把是词表里的一条，⛔ 算两处）⇒ "一句话里两把键只兑现一把"这类半承诺⛔ 许静默过关 |
//! | ⑦（注册表仍在） | `status-bar-shortcut-tip` 仍在 `src/elements.rs` 的登记面里 ⇒ 整格被删掉（承诺一起消失）也会红 |
//! | ⑧（能力层读数） | ⚠ **本票抓到的真实过承诺**：`[ ]` 那两把**键已绑、动作无落地实现**（`host::action_has_implementation` 只把这两个变体判 `false`），按下去什么都不会发生 ⇒ ⑧ 逐把钉住"有没有实现"这条读数，把它从注释升级成**判据名**（口径同 `d_ui_26` 的 `the_backslash_chord_stays_unimplemented_and_unpromised`，区别在那把键⛔ 绑、这两把绑了却死） |
//!
//! ## 边界（如实登记，⛔ 假装覆盖）
//!
//! 1. 本守卫只查**这一句**。其它"承诺但非键位"的文案（`device_rack.slint` 的 EQ 频响占位块、
//!    `arrangement_view.slint` 的波形占位块）是另一族缺口，⛔ 在本文件口径内。
//! 2. 本句**少**承诺是另一回事：键表今天绑了 20+ 把（`R` 录音、`M`/`S` 静音/独奏、`Delete`、
//!    `B` 铅笔、`Z` 缩放、`Shift+Enter` 采纳、`Cmd/Ctrl+Alt+A`、`F5`–`F9`、四向键……），
//!    这句话只印了 4 段。⚠ 扩这句话＝**用户可见**改动 ⇒ `tests/golden/linux/` 的基准确定性过期
//!    ⇒ 走 `gates-manual.yml` 的 `gate=goldens` ＋ 人工复核（`HD-56`），⛔ 本线自行决定；
//!    已作为待裁项登记进 `docs/ledger/app-zimmer-ui-notes.md`。
//! 3. `Focus::Other` 是本仓库"非画布、非文本域"的那一格（`src/input.rs` 的 `Focus` 只有三类）。
//!    真实窗口里"焦点在推子上"与"焦点在标签上"都归它 ⇒ ③ 的读数粒度到此为止。
//! 4. ⚠ **本守卫⛔ 不担保这句话"按了有用"** —— 它只担保"印出来的键在键表里真绑"。
//!    ⑧ 实测出这句话里有一处**能力层过承诺**（`[ ]` 两把键绑了、动作⛔ 有落地实现），
//!    修法有两个方向且**都⛔ 归人类裁决**：补 `ARCH-RT-005` 的 A/B 并发试听能力，
//!    或删那一段文案（用户可见 ⇒ `gate=goldens` 重录基准 + 人工复核，`HD-56`）。
//!    本切片两样都⛔ 做，只把它钉成读数并登记待裁（`docs/ledger/app-zimmer-ui-notes.md` §47）。
//!
//! ## 本机不编译重依赖（`AGENTS.md` §5）
//!
//! 本文件⛔ 装配 Slint 平台（源码扫描 ＋ 纯逻辑判据）。`input.rs` 本身是**零依赖状态机**
//! ⇒ 本机把它原样搬进 `/tmp` 桩 crate（`src/input.rs` = 真源码截到内部 `#[cfg(test)]` 之前、
//! `host.rs` 桩 = 逐字抄 `host.rs` 里 `action_has_implementation` 的函数体、`include_str!`
//! 指向那四个真文件的**副本**）后，**八条判据全部真跑**：基线 8/8 绿，六条变异检验各自红在
//! 该红的判据上（读数见台账 §47）。
//!
//! ⚠ 桩的读数只证**本文件自己的逻辑**（解析器、对账口径、断言方向），⛔ 当真 crate 已编译：
//! 桩里 `yeban_app` 被换成 `d27check`，那是另一份编译单元，键表/宿主 API 若有签名漂移
//! 只有 CI 看得见（`§45` 那条 `SharedString` 的教训就是它教的）。

use yeban_app::host::action_has_implementation;
use yeban_app::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution, Tool};

/// 承诺源：状态栏组件源码（`include_str!` ⇒ 文件被删/改名是**编译期**就红）。
const STATUS_BAR: &str = include_str!("../ui/status_bar.slint");
/// 主窗口源码：`StatusBar` 实例要是开始传 `tip`，本文件扫的默认值就成了死字面量。
const APP: &str = include_str!("../ui/app.slint");
/// 宿主源码：`set_tip(` 的命中数在这里数。
const HOST: &str = include_str!("../src/host.rs");
/// 登记面：语义 ID ↔ `.slint` 的双向契约。
const ELEMENTS: &str = include_str!("../src/elements.rs");

/// 分隔符（中点）是**字面量里的字节**，⛔ 用 ASCII 猜。
///
/// ⚠ 数字区间的连接号（en-dash `U+2013`）**只**以 `"\u{2013}"` 转义住在 `KEY_VOCAB` /
/// `PROMISES` 的字面量里，⛔ 这里不再提升一个裸字符常量：本文件的匹配靠的是词表**逐字**
/// 比对字面量，多一个无人读的常量只会撞 `-D warnings` 的 `dead_code`。
const SEGMENT_SEPARATOR: char = '·';

/// 这一句**应当**有几段（单位：段）—— 空集防线：删掉一段承诺也会红在这里。
const EXPECTED_SEGMENTS: usize = 4;
/// 钉死的承诺总数（单位：把键）—— `[ ]` 一段两把、`1–5` 一段五把。
const EXPECTED_PROMISES: usize = 9;

/// 键名词表：界面上的**写法** → 它代表的逻辑键（数组顺序＝前缀匹配顺序，长名在前）。
///
/// ⚠ 词表⛔ 认识某个写法 ⇒ 那一段判红（⑥/① 里的"未知段"），口径取自 `D-UI-26` ⑦：
/// "认不出就当它不是承诺"会**静默漏网**。
const KEY_VOCAB: &[(&str, &[LogicalKey])] = &[
    (
        // `1–5 工具`：五把数字键共用一句承诺。
        "1\u{2013}5",
        &[
            LogicalKey::Character('1'),
            LogicalKey::Character('2'),
            LogicalKey::Character('3'),
            LogicalKey::Character('4'),
            LogicalKey::Character('5'),
        ],
    ),
    // `[ ] A/B 盲听`：一段两把（左 = 主版本，右 = 提案版）。
    (
        "[ ]",
        &[LogicalKey::Character('['), LogicalKey::Character(']')],
    ),
    ("空格", &[LogicalKey::Space]),
    ("Tab", &[LogicalKey::Tab]),
];

/// 一条**钉死的**承诺：界面对某把键说的原话、它带的限定语、以及它必须兑现成什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Promise {
    /// 逻辑键（与 `KEY_VOCAB` 给出的那一个同源）。
    key: LogicalKey,
    /// 键名在界面上的**写法**（逐字）—— ① 靠它把字面量与本表对齐。
    spelling: &'static str,
    /// 这句承诺对这把键说的**原话**（逐字）。改文案＝改承诺 ⇒ 必须回来重新钉。
    said: &'static str,
    /// 文案带的**焦点限定语**（`None` = 文案说它哪儿都生效）。
    qualifier: Option<&'static str>,
    /// 期望的处置结果。
    expected: Resolution,
}

/// 钉死的九把（`[ ]` 与 `1–5` 各自展开）。
const PROMISES: &[Promise] = &[
    Promise {
        key: LogicalKey::Space,
        spelling: "空格",
        said: "播放/暂停",
        qualifier: None,
        expected: Resolution::Action(Action::PlayPause),
    },
    Promise {
        key: LogicalKey::Tab,
        spelling: "Tab",
        said: "切视图",
        // ⚠ 这一条是**唯一**带限定语的：键表里 `Tab` 真的只画布生效（`input.rs` 第 6 步），
        // 文案如实写了 ⇒ ③ 双向对账的两半之一就是它。
        qualifier: Some("画布聚焦时"),
        expected: Resolution::Action(Action::ToggleView),
    },
    Promise {
        key: LogicalKey::Character('1'),
        spelling: "1\u{2013}5",
        said: "工具",
        qualifier: None,
        expected: Resolution::Action(Action::SelectTool(Tool::Select)),
    },
    Promise {
        key: LogicalKey::Character('2'),
        spelling: "1\u{2013}5",
        said: "工具",
        qualifier: None,
        expected: Resolution::Action(Action::SelectTool(Tool::Pencil)),
    },
    Promise {
        key: LogicalKey::Character('3'),
        spelling: "1\u{2013}5",
        said: "工具",
        qualifier: None,
        expected: Resolution::Action(Action::SelectTool(Tool::Knife)),
    },
    Promise {
        key: LogicalKey::Character('4'),
        spelling: "1\u{2013}5",
        said: "工具",
        qualifier: None,
        expected: Resolution::Action(Action::SelectTool(Tool::Velocity)),
    },
    Promise {
        key: LogicalKey::Character('5'),
        spelling: "1\u{2013}5",
        said: "工具",
        qualifier: None,
        expected: Resolution::Action(Action::SelectTool(Tool::Eraser)),
    },
    Promise {
        key: LogicalKey::Character('['),
        spelling: "[ ]",
        said: "A/B 盲听",
        qualifier: None,
        expected: Resolution::Action(Action::AuditionMain),
    },
    Promise {
        key: LogicalKey::Character(']'),
        spelling: "[ ]",
        said: "A/B 盲听",
        qualifier: None,
        expected: Resolution::Action(Action::AuditionProposal),
    },
];

/// 解析出来的一条承诺（与钉死的 `Promise` 同形，但字段是**运行时**从字面量里取的）。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    key: LogicalKey,
    spelling: String,
    said: String,
    qualifier: Option<String>,
}

/// 把源码压成**只有代码行**：注释里的同名字面量不算实现站点（口径同 `D-UI-23`）。
fn code_only(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 取 `in property <string> <name>: "<literal>";` 的默认值（去首尾引号）。
///
/// 只认**行首**的声明；找不到声明本身即判据失败 —— "属性被改名/被删"比"文案漂移"更严重，
/// ⛔ 当"没找到就跳过"（口径同 `tests/status_bar_honesty.rs`）。
fn default_literal(source: &str, property: &str) -> String {
    let needle = format!("in property <string> {property}:");
    let line = source
        .lines()
        .map(str::trim_start)
        .find(|line| line.starts_with(&needle))
        .unwrap_or_else(|| {
            panic!("`ui/status_bar.slint` 里找不到 `{needle}` —— 属性被改名或删掉了")
        });
    let value = line
        .split_once(&needle)
        .expect("上面刚用同一个 needle 匹配过")
        .1
        .trim()
        .trim_end_matches(';')
        .trim();
    let literal = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("`{needle}` 的默认值必须是双引号字符串字面量；实际 {value:?}"));
    literal.to_owned()
}

/// 一段文案 → 若干条承诺（一把键一条）。返回 `None` = 这一段的键名**词表不认识**。
fn parse_segment(segment: &str) -> Option<Vec<Parsed>> {
    let (spelling, keys) = KEY_VOCAB
        .iter()
        .find(|(name, _)| segment.starts_with(name))?;
    let rest = segment[spelling.len()..].trim();
    // 限定语：`(...)` 里的那串；括号后的段尾⛔ 允许再有别的键名（⑥ 查这个）。
    let (said, qualifier) = match rest.find('(') {
        Some(at) => {
            let close = rest.find(')')?;
            (
                rest[..at].trim().to_owned(),
                Some(rest[at + 1..close].trim().to_owned()),
            )
        }
        None => (rest.to_owned(), None),
    };
    Some(
        keys.iter()
            .map(|key| Parsed {
                key: *key,
                spelling: (*spelling).to_owned(),
                said: said.clone(),
                qualifier: qualifier.clone(),
            })
            .collect(),
    )
}

/// 从字面量解析出全部承诺，连同"词表不认识的段"一起返回。
fn parse_tip(tip: &str) -> (Vec<Parsed>, Vec<String>) {
    let mut parsed = Vec::new();
    let mut unknown = Vec::new();
    for raw in tip.split(SEGMENT_SEPARATOR) {
        let segment = raw.trim();
        if segment.is_empty() {
            continue;
        }
        match parse_segment(segment) {
            Some(promises) => parsed.extend(promises),
            None => unknown.push(segment.to_owned()),
        }
    }
    (parsed, unknown)
}

/// 某段里**词表键名写法**出现的次数（⑥：一段只许有一个键名来源）。
fn spelling_hits_in(segment: &str, spelling: &str) -> usize {
    segment.match_indices(spelling).count()
}

/// 取 `StatusBar {` 那个实例的**块体**（花括号配对；找不到即判据失败）。
///
/// ⚠ 全程按 **`char` 计**（源码压成 `Vec<char>`）：`needle.len()` 是**字节**数，
/// 拿它去索引 `char` 数组会错格 —— 本仓库的 `.slint` 里全是中文注释，两种单位⛔ 混用。
fn component_block(source: &str, header: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let needle: Vec<char> = header.chars().collect();
    let start = chars
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
        .unwrap_or_else(|| panic!("源码里找不到 `{header}`"));
    let open = start + needle.len() - 1; // `StatusBar {` 的那个 '{'
    let mut depth = 0;
    let mut out = String::new();
    for &ch in &chars[open..] {
        if ch == '{' {
            depth += 1;
        } else if ch == '}' {
            depth -= 1;
        }
        out.push(ch);
        if depth == 0 {
            return out;
        }
    }
    panic!("`{header}` 的块体没有闭合");
}

/// 把一把键在指定焦点下喂给**那一张**键表。
fn resolve_bare(key: LogicalKey, focus: Focus) -> Resolution {
    let mut context = InputContext::new();
    context.set_focus(focus);
    context.resolve_logical(key, Modifiers::default())
}

/// ①（清点）：字面量解析出的承诺多重集逐字等于钉死的九把；段数与未知段也钉住。
#[test]
fn the_tip_line_promises_exactly_the_pinned_keys() {
    let code = code_only(STATUS_BAR);
    let tip = default_literal(&code, "tip");
    let segments: Vec<&str> = tip
        .split(SEGMENT_SEPARATOR)
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .collect();
    assert_eq!(
        segments.len(),
        EXPECTED_SEGMENTS,
        "① 这句话的段数变了（单位：段，实际 {} ⇒ {segments:?}）—— 删一段承诺就是把一条用户可见的键位承诺抹掉",
        segments.len()
    );

    let (parsed, unknown) = parse_tip(&tip);
    assert!(
        unknown.is_empty(),
        "① 有 {unknown:?} 段的键名⛔ 在词表里（单位：段）—— 认不出就当它不是承诺会**静默漏网**；\
         真加了一把键就同步扩 KEY_VOCAB 与 PROMISES",
    );
    assert_eq!(
        parsed.len(),
        EXPECTED_PROMISES,
        "① 承诺总数漂移（单位：把，实际 {}）—— `[ ]` 一段两把、`1\u{2013}5` 一段五把",
        parsed.len()
    );

    // 逐条对账：键、写法、原话、限定语四样都必须是钉死的那一份。
    let mut seen: Vec<String> = parsed.iter().map(parsed_promise_id).collect();
    seen.sort();
    let mut want: Vec<String> = PROMISES.iter().map(Promise::promise_id).collect();
    want.sort();
    assert_eq!(
        seen, want,
        "① 这句话承诺的键位集合与钉死的清单⛔ 一致（单位：把）—— 上面是现读、下面是钉值"
    );
}

impl Promise {
    /// 这条承诺的**对账身份串**（口径与 `parsed_promise_id` 逐字相同，只是字段来自钉死的表）。
    fn promise_id(&self) -> String {
        promise_id(self.spelling, self.key, self.said, self.qualifier)
    }

    /// 钉住的处置结果里那个 [`Action`]（⑧ 问能力层要用）。
    ///
    /// ⚠ **穷尽匹配**、⛔ `_` 通配：本表的每一条承诺按定义必须是 `Resolution::Action`，
    /// 将来 `Resolution` 加变体时这里会在**编译期**被逼表态。
    fn expected_action(&self) -> Action {
        match self.expected {
            Resolution::Action(action) => action,
            Resolution::PassThrough | Resolution::ConsumedByIme => {
                panic!(
                    "钉死的承诺 {:?} 不是动作 —— 「印出来的键放行」不成其为用户可见承诺",
                    self.key
                )
            }
        }
    }
}

/// 对账用的身份串：键名写法 + 逻辑键 + 原话 + 限定语。
fn promise_id(spelling: &str, key: LogicalKey, said: &str, qualifier: Option<&str>) -> String {
    format!(
        "{spelling}|{key:?}|{said}|{}",
        qualifier.unwrap_or("\u{2205}")
    )
}

/// 解析出来的一条承诺 → 同一个身份串（⛔ 给 `Parsed` 造 `Promise` 同形的构造器：
/// 本文件两型不可互转，字段一个是 `&'static str` 一个是 `String`，构造出来反而更绕）。
fn parsed_promise_id(promise: &Parsed) -> String {
    promise_id(
        &promise.spelling,
        promise.key,
        &promise.said,
        promise.qualifier.as_deref(),
    )
}

/// ②（兑现·画布）：九把键在画布焦点下都解析出期望的那个 `Action`。
#[test]
fn every_promised_key_resolves_to_the_pinned_action_on_the_canvas() {
    for promise in PROMISES {
        assert_eq!(
            resolve_bare(promise.key, Focus::MainCanvas),
            promise.expected,
            "② 画布聚焦下，界面把 {} 说成「{}」，键表却⛔ 解析成那个动作（实际 {:?}）",
            promise.spelling,
            promise.said,
            resolve_bare(promise.key, Focus::MainCanvas)
        );
    }
}

/// ③（兑现·Other ＋ 限定语双向对账）：**文案里**的限定语与**键表里**的门必须同进同退。
///
/// ⚠ 本判据读的是**解析出来的**限定语（`parsed`），⛔ 钉死表里那一份 —— 钉表与文案的对账
/// 归 ①（身份串里就带限定语）。这里若读钉表，"把 `(画布聚焦时)` 从文案里删掉"会**只红在 ①**，
/// 而 ③ 这个名字会绿着说谎（实测：删掉限定语后 ③ 的旧写法照样通过）。
#[test]
fn the_focus_qualifier_in_the_text_matches_the_gate_in_the_key_table() {
    let code = code_only(STATUS_BAR);
    let tip = default_literal(&code, "tip");
    let (parsed, unknown) = parse_tip(&tip);
    assert!(
        unknown.is_empty(),
        "③ 有 {unknown:?} 段读不出来（单位：段）—— 本判据要按**文案**里的限定语对账，\
         词表先扩了才能走到这一步"
    );

    for promise in PROMISES {
        let text_qualifier = parsed
            .iter()
            .find(|entry| entry.key == promise.key)
            .unwrap_or_else(|| {
                panic!(
                    "③ 这把键 {:?} 已不在文案的承诺里 —— 承诺被删是**用户可见**改动，先按 ① 的红处理",
                    promise.key
                )
            })
            .qualifier
            .clone();

        match text_qualifier {
            // 文案写了限定语 ⇒ 键表**必须**在非画布焦点下⛔ 吞这把键（否则文案在少报能力）。
            Some(qualifier) => {
                assert_eq!(
                    qualifier, "画布聚焦时",
                    "③ 文案里的限定语措辞漂移（实际 {qualifier:?}）—— 换措辞就一并更新本表与 ①"
                );
                assert_eq!(
                    resolve_bare(promise.key, Focus::Other),
                    Resolution::PassThrough,
                    "③ 文案说「{}」只在{qualifier}生效，键表却在 Other 焦点下也吞了这把键（实际 {:?}）",
                    promise.spelling,
                    resolve_bare(promise.key, Focus::Other)
                );
            }
            // 文案⛔ 写限定语 ⇒ 它承诺的是"哪儿都生效" ⇒ Other 那条腿也必须兑现。
            None => {
                assert_eq!(
                    resolve_bare(promise.key, Focus::Other),
                    promise.expected,
                    "③ 这是对「哪儿都生效」的承诺，但键表在 Other 焦点下解析成 {:?}（期望 {:?}）—— \
                     要么文案补回限定语（用户可见改动，走 `gate=goldens`），要么键表放开这一格",
                    resolve_bare(promise.key, Focus::Other),
                    promise.expected
                );
            }
        }
    }
}

/// ④（文本域口径）：九把键在文本域聚焦下全部放行 —— 这是**现状**，⛔ 算兑现。
#[test]
fn the_tip_line_says_nothing_about_text_fields_and_the_table_passes_through() {
    let mut context = InputContext::new();
    context.set_focus(Focus::TextInput);
    for promise in PROMISES {
        assert_eq!(
            context.resolve_logical(promise.key, Modifiers::default()),
            Resolution::PassThrough,
            "④ 文本域聚焦下 {} 被 DAW 吞成了 {:?} —— 文本域整体放行是 ADR-0068 的**未决**现状，\
             这句话⛔ 对它作任何承诺",
            promise.spelling,
            context.resolve_logical(promise.key, Modifiers::default())
        );
    }
}

/// ⑤（画的 == 扫的）：那串默认值必须真的上屏，且⛔ 有第二个写者。
#[test]
fn the_scanned_literal_is_the_one_drawn_and_has_no_second_writer() {
    let code = code_only(STATUS_BAR);
    assert!(
        code.contains("text: root.tip;"),
        "⑤ `status_bar.slint` 里那句 `text: root.tip;` 不在了 ⇒ 本文件扫的字面量可能根本没上屏"
    );

    let block = component_block(&code_only(APP), "StatusBar {");
    assert!(
        !block.contains("tip:"),
        "⑤ `app.slint` 的 `StatusBar` 实例开始传 `tip:` 了 ⇒ 界面画的是那一份，\
         本文件扫的 `.slint` 默认值成了死字面量（块体：{block}）"
    );

    assert_eq!(
        code_only(HOST).matches("set_tip(").count(),
        0,
        "⑤ 宿主里出现了 `set_tip(` 写者（单位：处）—— 那 `tip` 就不再是静态承诺，\
         本守卫的口径要连带改成扫写者的取值（现读 {} 处）",
        code_only(HOST).matches("set_tip(").count()
    );
}

/// ⑥（一段⛔ 两把键）：每段只许有一个词表键名来源。
#[test]
fn each_segment_promises_exactly_one_key_spelling() {
    let code = code_only(STATUS_BAR);
    let tip = default_literal(&code, "tip");
    for raw in tip.split(SEGMENT_SEPARATOR) {
        let segment = raw.trim();
        if segment.is_empty() {
            continue;
        }
        let hits: Vec<(&str, usize)> = KEY_VOCAB
            .iter()
            .map(|(name, _)| (*name, spelling_hits_in(segment, name)))
            .filter(|(_, count)| *count > 0)
            .collect();
        assert_eq!(
            hits.len(),
            1,
            "⑥ 段 {segment:?} 里有 {hits:?} 个词表键名来源 —— 一句话里两把键只兑现一把，\
             就是本票要抓的半承诺形状（把它拆成两段，承诺才各自可验）"
        );
        assert_eq!(
            hits[0].1, 1,
            "⑥ 段 {segment:?} 里写法 {:?} 出现了 {} 次（单位：次）—— 同上",
            hits[0].0, hits[0].1
        );
    }
}

/// ⑦（注册表仍在）：语义 ID 还在登记面里 ⇒ 整格被删也会红。
#[test]
fn the_tip_cell_is_still_registered_for_the_control_plane() {
    assert!(
        code_only(ELEMENTS).contains("\"status-bar-shortcut-tip\""),
        "⑦ `src/elements.rs` 的登记面里找不到 `status-bar-shortcut-tip` —— 这一格被删/改名，\
         承诺一起消失（`[UI-TEST-001]` 的双向契约）"
    );
}

/// ⑧（能力层读数）：这句承诺里，**哪几把键按下去真的会发生事情**。
///
/// ⚠ 本票**抓到一处过承诺并如实把它钉成读数**（⛔ 掩盖，也⛔ 顺手删文案）：
/// `[ ] A/B 盲听` 那两把键**已绑**（② 绿）但**动作无落地实现** —— `[ARCH-RT-005]` 要求的
/// "主线与提案分支并发渲染 + 30ms 等功率瞬切 + 2048 采样预滚"在仓库里一处都不存在，
/// `host::action_has_implementation` 今天只把这两个变体判 `false`（其余全 `true`）。
///
/// 为什么不在此切片里删掉那一段：那句话是**画在屏幕上的**用户可见文本 ⇒ `tests/golden/linux/`
/// 的基准确定性过期 ⇒ 必须走 `gates-manual.yml` 的 `gate=goldens` ＋ 人工复核（`HD-56`）。
/// 已作为待裁项登记进 `docs/ledger/app-zimmer-ui-notes.md`。
///
/// 三条腿各自会把什么读红，写在断言消息里 ⇒ 将来无论"补上 A/B 能力"还是"改文案"，
/// 都必须回到本文件重新对账，⛔ 一边悄悄漂。
#[test]
fn the_tip_promises_two_bound_but_dead_keys_and_that_over_promise_is_pinned_as_a_reading() {
    // ⚠ 直接比 `LogicalKey` 值，⛔ 把它 `format!` 成字符串再比 —— `{:?}` 出来的字符引号
    // 形状（`Character('[')`）是人眼对着字符串猜，值比对才是判据。
    let dead: Vec<LogicalKey> = PROMISES
        .iter()
        .filter(|promise| !action_has_implementation(promise.expected_action()))
        .map(|promise| promise.key)
        .collect();

    assert_eq!(
        dead,
        vec![LogicalKey::Character('['), LogicalKey::Character(']')],
        "⑧ 承诺与能力层的读数变了（单位：把；现读 {dead:?}）—— 下面钉的是今天的事实。\n\
         ⚠ 若变多：新绑了一把⛔ 有落地的键却印进了这句话 ⇒ 过承诺扩大，本票口径要跟着改；\n\
         ⚠ 若变少（A/B 盲听被实现）：⭐ 那是好消息，但这句承诺必须**同步升级**，\
         并⛔ 再留着「键已绑却死」这一格。"
    );

    // 反面事实：那一段此刻**还在**这句话里 —— ⑧ 抓到的过承诺是"文案 + 键表 + 能力"三层的交集，
    // 任何一层单独漂都不算修复（删文案是用户可见改动，⛔ 由本线自行决定）。
    let code = code_only(STATUS_BAR);
    let tip = default_literal(&code, "tip");
    assert!(
        tip.contains("[ ]"),
        "⑧ 这句话里的 `[ ]` 段不见了 —— 若那是**人工裁决后改的文案**（过承诺已消除），\
         本票要一起改写：⑧ 降级成「九把全有实现」并删掉上面那条待裁登记；\
         若⛔ 通知本线地顺手删掉，那就是把用户可见承诺悄悄抹了（现读 {tip:?}）"
    );
}
