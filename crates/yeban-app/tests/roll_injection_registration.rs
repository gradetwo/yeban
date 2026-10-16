//! `[ROAD-M3-002]` 宿主**音符数组注入面**的登记判据（任务单 `#127`；台账第 283 条）。
//!
//! ## 这个文件存在的理由
//!
//! `host.rs::apply_view` 的注释曾写"**六个**平行数组（… / `rows`）必须共用同一索引集"，
//! 而实现只注入 **5** 个（`ulids` / `velocities` / `positions` / `widths` / `ys`）＋ 第 6 条
//! `note-selected`；`VisibleNotes::rows` 的**宿主生产读点 = 0**（台账第 283 条的命中）。
//! 本文件把"注释与实现一致"与"`rows` 有意不消费"各钉成**从 `host.rs` / `bridge.rs`
//! 源码派生**的事实。
//!
//! ## 口径（三个断言数三个对象）
//!
//! | 断言 | 数的对象 | 单位 |
//! | :--- | :--- | :--- |
//! | ① | `apply_view` 注入窗口里 `ui.set_note_<名>(…)` 的名字集合（按来源分两档） | 个名字 |
//! | ② | 同一窗口（去掉注释行）里标识符 `rows` 的出现次数 | 次 |
//! | ③ | `visible_notes` 结果的每个绑定名上 `.<绑定名>.rows` 的读点 | 个 |
//!
//! ## 第 293 条（本条判据的构造约束）
//!
//! ⛔ **不从本文件读**被判的字符串：三个被测对象都在 `../src/host.rs` 与 `../src/bridge.rs`
//! 里，本文件只提供**解析**（锚点 ＋ 窗口 ＋ 重算），不提供文本。

use std::collections::BTreeSet;

/// 被核对的宿主源码 —— ⛔ 不是本文件（台账第 293 条）。
const HOST_RS: &str = include_str!("../src/host.rs");
/// 被核对的投影源码（`VisibleNotes::rows` 的登记面）—— ⛔ 不是本文件（台账第 293 条）。
const BRIDGE_RS: &str = include_str!("../src/bridge.rs");

/// 注入块的起点：`apply_view` 里唯一那次"按视口取音符"。
const INJECT_ANCHOR: &str = "let visible = view.visible_notes(scroll_x, viewport_width);";
/// 注入块的终点（下一个与音符无关的注入）—— 窗口是**半开**的。
const INJECT_END: &str = "ui.set_track_colors(";
/// 被判的那段注释的起点。
const COMMENT_ANCHOR: &str = "// `[ROAD-M3-002 / BASELINE-003]` **视口裁剪**";

/// `[ROAD-M3-002 / BASELINE-003]` 视口裁剪**必须**注入的 5 个平行数组（派生期望值）。
const INJECTED_NOTE_ARRAYS: [&str; 5] = ["ulids", "velocities", "positions", "widths", "ys"];

/// 换行计数 ⇒ 1 基行号。
fn line_of(source: &str, byte: usize) -> usize {
    source[..byte].matches('\n').count() + 1
}

/// 标识符 `token` 在 `text` 里的**词边界**命中次数（`track_rows` 这样的词不算）。
fn token_hits(text: &str, token: &str) -> usize {
    let bytes = text.as_bytes();
    let mut hits = 0;
    let mut from = 0;
    while let Some(rel) = text[from..].find(token) {
        let at = from + rel;
        let before_ok = at == 0 || !is_word(char::from(bytes[at - 1]));
        let after = at + token.len();
        let after_ok = after == bytes.len() || !is_word(char::from(bytes[after]));
        if before_ok && after_ok {
            hits += 1;
        }
        from = at + token.len();
    }
    hits
}

/// 标识符字符（ASCII 口径：判据核对的都是 ASCII 名）。
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// 注入窗口原文（**含注释行**）—— 窗口是半开的：`[锚点, ui.set_track_colors(` )。
fn raw_injection_window() -> &'static str {
    let start = HOST_RS
        .find(INJECT_ANCHOR)
        .expect("host.rs 里找不到 `apply_view` 的音符注入锚点（改名 ⇒ 本判据必须跟着改）");
    let rest = &HOST_RS[start..];
    let end = rest
        .find(INJECT_END)
        .expect("host.rs 里找不到注入块的终点 `ui.set_track_colors(`");
    &rest[..end]
}

/// 注入窗口：`(首行, 末行, 去掉注释行后的代码)`。
fn injection_code_window() -> (usize, usize, String) {
    let start = HOST_RS
        .find(INJECT_ANCHOR)
        .expect("host.rs 里找不到 `apply_view` 的音符注入锚点（改名 ⇒ 本判据必须跟着改）");
    let raw = raw_injection_window();
    let code = raw
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    (
        line_of(HOST_RS, start),
        line_of(HOST_RS, start + raw.len()) - 1,
        code,
    )
}

/// 注释块：`(首行, 末行, 原文)`。
fn comment_window() -> (usize, usize, &'static str) {
    let start = HOST_RS.find(COMMENT_ANCHOR).expect("注释锚点不见了");
    let end = HOST_RS.find(INJECT_ANCHOR).expect("注入锚点不见了");
    assert!(start < end, "注释必须紧邻注入块之前");
    (
        line_of(HOST_RS, start),
        line_of(HOST_RS, end) - 1,
        &HOST_RS[start..end],
    )
}

/// 中文数词（1..=9）—— 用来把注释里的"几个平行数组"**换算**成派生数量，⛔ 不钉字面量。
fn chinese_numeral(n: usize) -> &'static str {
    match n {
        1 => "一",
        2 => "二",
        3 => "三",
        4 => "四",
        5 => "五",
        6 => "六",
        7 => "七",
        8 => "八",
        9 => "九",
        _ => panic!("本判据只覆盖 1..=9：现在数到 {n}"),
    }
}

/// **断言 ①②**：注释声明的数组集合 ＋ 数量，必须与实现注入的逐字一致。
#[test]
fn host_comment_and_injection_agree_on_the_note_array_set() {
    let (first, last, code) = injection_code_window();

    let mut injected: BTreeSet<String> = BTreeSet::new();
    let mut siblings: Vec<String> = Vec::new();
    for line in code.lines() {
        let trimmed = line.trim_start();
        let Some(rest) = trimmed.strip_prefix("ui.set_note_") else {
            continue;
        };
        let open = rest.find('(').expect("`ui.set_note_*` 必须带左括号");
        let name = rest[..open].to_string();
        match trimmed.find("&visible.") {
            Some(at) => {
                let field: String = trimmed[at + "&visible.".len()..]
                    .chars()
                    .take_while(|c| is_word(*c))
                    .collect();
                assert_eq!(
                    name, field,
                    "`ui.set_note_{name}` 注入的必须是 `visible.{field}`（同索引集 ⇒ 名字一一对应）"
                );
                injected.insert(name);
            }
            None => siblings.push(name),
        }
    }

    let expected: BTreeSet<String> = INJECTED_NOTE_ARRAYS
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    assert_eq!(
        injected, expected,
        "`apply_view` 从 `visible_notes` 注入的数组集合必须是 {expected:?}；现读 {injected:?}"
    );
    assert_eq!(
        siblings,
        vec!["selected".to_string()],
        "第 6 条注入是 `note-selected`（由选区现算，⛔ 不是 `rows`）"
    );
    assert_eq!(
        token_hits(&code, "rows"),
        0,
        "`host.rs:{first}-{last}` 的注入窗口里⛔ 不得出现标识符 `rows`（有意不注入）"
    );

    let (cfirst, clast, comment) = comment_window();
    // 注释里那个用 ` / ` 分隔的清单 —— 派生，而不是钉整句。
    let list = comment
        .match_indices('（')
        .find_map(|(i, _)| {
            let tail = &comment[i + '（'.len_utf8()..];
            let close = tail.find('）')?;
            let inner = &tail[..close];
            if inner.contains(" / ") {
                Some(inner.to_string())
            } else {
                None
            }
        })
        .expect("注释里必须有那个用 ` / ` 分隔的数组清单");
    let declared: BTreeSet<String> = list.split(" / ").map(|s| s.trim().to_string()).collect();
    assert_eq!(
        declared, injected,
        "注释 `host.rs:{cfirst}-{clast}` 声明的数组集合必须与实现注入的一致（现读注释 {declared:?} / 实现 {injected:?}）"
    );
    assert!(
        comment.contains(&format!("{}个平行数组", chinese_numeral(injected.len()))),
        "注释必须把派生数量写成『{}个平行数组』（台账第 283 条：解析了 ≠ 有消费者）",
        chinese_numeral(injected.len())
    );
    assert!(
        !comment.contains("六个平行数组"),
        "注释不得再自称『六个平行数组』—— `rows` 不在注入面里"
    );
    assert!(
        comment.contains("`VisibleNotes::rows`") && comment.contains("有意不注入"),
        "注释必须点名 `rows` 与它的处置（有意不注入）"
    );
    // 兄弟注入的**序数**也必须由派生数量算出：曾写"第七个" ⇒ 与"六个数组"是同一个错
    // （把不注入的 `rows` 也算进了索引集）。
    let raw = raw_injection_window();
    let ordinal = format!("（第{}个", chinese_numeral(injected.len() + 1));
    assert!(
        raw.contains(&ordinal),
        "`note-selected` 是五个数组之后的那一个 ⇒ 注释必须写 `{ordinal}…）`（派生：5 + 1 = {}）",
        injected.len() + 1
    );
    assert!(
        !raw.contains("（第七个"),
        "⛔ 不得再写『第七个』—— 那是把 `rows` 算进索引集的旧口径"
    );
    println!(
        "[roll-injection] 注入块 host.rs:{first}-{last}；从 `visible_notes` 注入 {} 个名字 {injected:?}；同索引集兄弟注入 {siblings:?}；注释 host.rs:{cfirst}-{clast} 声明 {declared:?}；序数写 `{ordinal}`",
        injected.len()
    );
}

/// **断言 ③**：`visible_notes` 的每个绑定名上，`.rows` 的宿主导入读点 = 0（第 283 条）。
#[test]
fn visible_rows_has_no_host_consumer() {
    let mut bindings: Vec<(usize, String)> = Vec::new();
    for (at, _) in HOST_RS.match_indices(".visible_notes(") {
        let line_start = HOST_RS[..at].rfind('\n').map_or(0, |i| i + 1);
        let line_end = HOST_RS[at..].find('\n').map_or(HOST_RS.len(), |i| at + i);
        let line = &HOST_RS[line_start..line_end];
        let head = line.split('=').next().unwrap_or("").trim();
        let name = head.strip_prefix("let ").unwrap_or(head).trim().to_string();
        assert!(
            !name.is_empty() && !name.contains(' '),
            "每个 `visible_notes` 调用点都必须绑定到一个简单名字（现读 `{line}`）"
        );
        bindings.push((line_of(HOST_RS, at), name));
    }
    assert!(
        !bindings.is_empty(),
        "host.rs 里必须至少有一处 `visible_notes` 调用点（否则本判据无从派生）"
    );

    for (line, name) in &bindings {
        assert_eq!(
            token_hits(HOST_RS, &format!("{name}.rows")),
            0,
            "`host.rs:{line}` 绑定的 `{name}` 不得读 `.rows`：`rows` 今天零宿主消费者，\
             按台账第 283 条登记为**有意不消费**（要给它消费者 ⇒ 先改 `bridge.rs` 的修订段与预算读数）"
        );
    }
    let host_code = HOST_RS
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(
        token_hits(&host_code, "VisibleNotes"),
        0,
        "宿主的**代码行**里不出现类型名 `VisibleNotes` ⇒ 也不可能用 `VisibleNotes {{ rows, .. }}` 解构它\
         （注释里点名登记不算消费）"
    );

    println!(
        "[roll-injection] `visible_notes` 调用点（行号, 绑定名）= {bindings:?}；这些绑定名上的 `.rows` 读点 = 0"
    );
}

/// **第 278 条**（保留原文 ＋ 追加修订段 ＋ 点名被修订的支）：`rows` 的登记本身可判。
#[test]
fn rows_is_registered_as_intentionally_unconsumed() {
    for original in [
        "`[ROAD-M3-002]` 裁剪后的**全部平行数组**（**同一索引集**）。",
        "/// 音高车道索引。",
    ] {
        assert!(
            BRIDGE_RS.contains(original),
            "第 278 条：`rows` 的原登记原文⛔ 不许删 —— 缺 `{original}`"
        );
    }

    let anchor = "追加修订段（任务单 `#127`";
    let at = BRIDGE_RS
        .find(anchor)
        .unwrap_or_else(|| panic!("`bridge.rs` 里必须逐字含修订段锚点 `{anchor}`"));
    let tail = &BRIDGE_RS[at..];
    let seg = &tail[..tail
        .find("\n    pub rows")
        .expect("修订段必须在 `rows` 字段声明之前")];
    for needle in [
        "只针对 `rows` 这一支",
        "有意不消费",
        "宿主生产读点 = 0",
        "第 283 条",
        "APPLY_VIEW_WRITE_SITE_BUDGET",
    ] {
        assert!(
            seg.contains(needle),
            "修订段必须可判地含 `{needle}`（第 278/280 条：修订段本身要能被逐字钉住）"
        );
    }
    println!(
        "[roll-injection] `bridge.rs` 的 `rows` 修订段起于第 {} 行，长 {} 字节；原文两句保留 = true",
        line_of(BRIDGE_RS, at),
        seg.len()
    );
}
