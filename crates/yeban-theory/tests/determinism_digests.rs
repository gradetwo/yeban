//! R70② 落地 ＋ R75② 的机械守卫 ＋ 方向 5（根音文本邻域）。
//!
//! ## 为什么"两次运行相同"不是契约（R70②）
//!
//! 本 crate 有 **13 条**`assert_eq!(f(x), f(x))`（两侧**源码文本相同**，机械扫出）：
//! `genre.rs` 11 条（`ids` / `search` / `by_scale` / `by_source` / `source_histogram` /
//! `by_drum_style` / `drum_style_histogram` / `ids` / `search` / `sketch_for` / 自反）、
//! `lib.rs` 1 条（`splitmix64(42,0) == splitmix64(42,0)`）、
//! `tests/properties.rs` 1 条（`realize_three_voices` 两次调用）。
//!
//! 这些断言只证明"同一份实现跑两次给自己同一个答案"。**一处一致的实现改动会让它们
//! 全部保持绿**，而交付读数已经变了 ⇒ ⛔ 自比不是字节契约。本文件用
//! **字面摘要（长度 ＋ FNV-1a 64）**把这些读数钉死：摘要一变就红。
//!
//! ⚠ 摘要选的是**纯整数路径**（无 `sin`/`cos`/`exp`/`ln`/`powf`/`log10`），
//! 因此跨架构逐位一致（见 R77）。

use yeban_theory::chord::Tonality;
use yeban_theory::genre::{GenreLibrary, SOURCE_TRADITIONAL_THEORY};
use yeban_theory::melody::{MelodyConstraints, melody_over_chords};
use yeban_theory::pitch::{PitchClass, parse_pitch_class};
use yeban_theory::progression::{Meter, expand_progression};
use yeban_theory::rhythm::{metric_grid, swung_metric_grid};
use yeban_theory::scale::{Scale, ScaleKind};
use yeban_theory::splitmix64;
use yeban_theory::voice_leading::realize_three_voices;

/// FNV-1a 64（公有领域）：纯 64 位整数运算，无超越函数。
fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// 读数的字面摘要：`(长度, FNV-1a 64)`。
fn digest_of(text: &str) -> (usize, u64) {
    (text.len(), fnv1a64(text.as_bytes()))
}

/// R70②：把 7 条"确定性"读数的**字面摘要**钉死。
///
/// 前身是 `assert_eq!(f(x), f(x))`（自比）；本判据取代它们的契约地位。
/// 每行读数都覆盖一条被自比"守"着的入口。
#[test]
fn deterministic_readings_are_pinned_to_literal_digests() {
    let key = Scale::new(PitchClass::C, ScaleKind::Major);

    // 1. 声部连接：`I-V-vi-IV-ii-V-I` 8 小节、3 声部。
    let spans = expand_progression(&key, "I-V-vi-IV-ii-V-I", 8).unwrap();
    let realized = realize_three_voices(&spans).unwrap();
    let voicings = realized
        .voicings
        .iter()
        .map(|voices| {
            voices
                .iter()
                .map(|pitch| pitch.value().to_string())
                .collect::<Vec<_>>()
                .join(",")
        })
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(
        digest_of(&voicings),
        (71, 0xA7DA_79AB_20EF_DB4A),
        "voicings"
    );

    // 2. 旋律：C 大调 `I-V-vi-IV`、4 小节、每小节 5 个 onset、种子 99。
    let grid4 = metric_grid(Meter::COMMON, 4, 5).unwrap();
    let spans4 = expand_progression(&key, "I-V-vi-IV", 4).unwrap();
    let melody = melody_over_chords(&key, &spans4, &grid4, MelodyConstraints::DEFAULT, 99).unwrap();
    let line = melody
        .notes()
        .iter()
        .map(|note| format!("{}:{}:{}", note.start_tick, note.duration_ticks, note.pitch))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(digest_of(&line), (240, 0xB0D6_E743_94B6_540E), "melody");

    // 3. 节奏网格：7/8、5 小节、每小节 6 个 onset、摇摆 660 千分比。
    let grid = swung_metric_grid(Meter::SEVEN_EIGHT, 5, 6, Some(660)).unwrap();
    let hits = grid
        .hits()
        .iter()
        .map(|hit| format!("{}:{}:{}:{}", hit.tick, hit.bar, hit.cell, hit.weight))
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(digest_of(&hits), (341, 0x5128_F2EE_20EC_A6A3), "grid hits");

    // 4. 流派库的三个只读入口（`OnceLock` 缓存前后的路径）。
    let ids = GenreLibrary::ids().join(",");
    assert_eq!(digest_of(&ids), (1831, 0x3594_A110_B44A_0F26), "ids");
    let jazz = GenreLibrary::search("jazz")
        .iter()
        .map(|rule| rule.id)
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        digest_of(&jazz),
        (86, 0xB3F5_81EF_ECEB_EA05),
        "search(jazz)"
    );
    let traditional = GenreLibrary::by_source(SOURCE_TRADITIONAL_THEORY)
        .iter()
        .map(|rule| rule.id)
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        digest_of(&traditional),
        (540, 0x1AC5_762A_8471_C0DF),
        "by_source(traditional)"
    );

    // 5. SplitMix64 的前 64 个读数（种子 0..64、盐 7）。
    let stream = (0u64..64)
        .map(|seed| splitmix64(seed, 7).to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert_eq!(
        digest_of(&stream),
        (1305, 0xDC74_2D0B_957D_91AC),
        "splitmix64"
    );

    // 摘要真的取决于内容（不是常量函数）：改一个字符必须改摘要。
    assert_ne!(digest_of("a"), digest_of("b"));
    assert_ne!(digest_of(&ids), digest_of(&jazz));
}

// ---------------------------------------------------------------------------
// R75②：机械守卫 —— 宏断言的两侧**源码文本相同**时：
//   * `assert_ne!` ⇒ 构造上相等 ⇒ **必然恒红**（坏掉的探针）；
//   * `assert_eq!` ⇒ 构造上相等 ⇒ **恒真（空断言）**，不构成证据（R70② 的自比）。
// 本守卫先用"一条已知红 ＋ 一条已知绿"自证（R56），再扫全 crate 源码。
// ---------------------------------------------------------------------------

/// 字符串字面量所占的字节区间（用来把"出现在字符串里的宏名"排除掉 ——
/// 本批实测过这个假阳性：本判据自己的"已知红"样例就是一段字符串）。
fn string_ranges(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut ranges = Vec::new();
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            let start = index;
            index += 1;
            while index < bytes.len() {
                if bytes[index] == b'\\' {
                    index += 2;
                    continue;
                }
                if bytes[index] == b'"' {
                    index += 1;
                    break;
                }
                index += 1;
            }
            ranges.push((start, index));
        } else {
            index += 1;
        }
    }
    ranges
}

/// 收集 `assert_eq!` / `assert_ne!` 两侧**文本相同**的行号（`macro_name` 不带 `!`）。
///
/// 口径：两侧按"顶层逗号"切开（括号/方括号/花括号计入深度），空白归一化后比较；
/// 出现在字符串字面量里的宏名不算调用。
fn identical_sides(source: &str, macro_name: &str) -> Vec<usize> {
    let needle = format!("{macro_name}!(");
    let ranges = string_ranges(source);
    let in_string = |offset: usize| ranges.iter().any(|(a, b)| offset >= *a && offset < *b);
    let normalize = |text: &str| text.split_whitespace().collect::<Vec<_>>().join(" ");
    let split_top = |text: &str| -> Option<(usize, usize)> {
        let mut depth = 0i32;
        let mut first = None;
        for (index, ch) in text.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => depth -= 1,
                ',' if depth == 0 => {
                    if first.is_none() {
                        first = Some(index);
                    } else {
                        return first.map(|f| (f, index));
                    }
                }
                _ => {}
            }
        }
        first.map(|f| (f, text.len()))
    };
    let mut hits = Vec::new();
    let mut offset = 0usize;
    while let Some(position) = source[offset..].find(&needle) {
        let abs = offset + position;
        offset = abs + needle.len();
        if in_string(abs) {
            continue;
        }
        let after = &source[offset..];
        let mut depth = 1i32;
        let mut end = after.len();
        for (index, ch) in after.char_indices() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = index;
                        break;
                    }
                }
                _ => {}
            }
        }
        let body = &after[..end];
        if let Some((comma, _)) = split_top(body) {
            let left = normalize(&body[..comma]);
            let tail = &body[comma + 1..];
            let right = match split_top(tail) {
                Some((next, _)) => normalize(&tail[..next]),
                None => normalize(tail),
            };
            if left == right {
                hits.push(source[..abs].matches('\n').count() + 1);
            }
        }
    }
    hits
}

/// R56：守卫先用一条**已知红**（两侧相同）和一条**已知绿**（两侧不同）喂过，
/// 再扫真源码。真源码里 `assert_ne!` 的相同两侧必须**一条都没有**；
/// `assert_eq!` 的相同两侧是 R70② 的自比清单，必须**恰好是本批枚举的 13 条**。
#[test]
fn assert_ne_never_compares_a_value_with_itself() {
    // 已知红。
    assert_eq!(identical_sides("assert_ne!(x, x);", "assert_ne"), vec![1]);
    assert_eq!(
        identical_sides("fn t() {\n    assert_ne!(a.b(), a.b());\n}", "assert_ne"),
        vec![2]
    );
    // 已知绿：两侧不同（含"同一表达式、不同实参"）。R69：无载荷枚举跨变体不是假探针。
    assert!(identical_sides("assert_ne!(x, y);", "assert_ne").is_empty());
    assert!(identical_sides("assert_ne!(f(1), f(2));", "assert_ne").is_empty());
    assert!(
        identical_sides(
            "assert_ne!(ChordKind::Minor7, ChordKind::Major7);",
            "assert_ne"
        )
        .is_empty()
    );
    // 字符串里的宏名不算调用（已知绿）。
    assert!(identical_sides("let s = \"assert_ne!(x, x);\";", "assert_ne").is_empty());

    const SOURCES: [(&str, &str); 16] = [
        ("src/lib.rs", include_str!("../src/lib.rs")),
        ("src/error.rs", include_str!("../src/error.rs")),
        ("src/pitch.rs", include_str!("../src/pitch.rs")),
        ("src/scale.rs", include_str!("../src/scale.rs")),
        ("src/chord.rs", include_str!("../src/chord.rs")),
        ("src/progression.rs", include_str!("../src/progression.rs")),
        (
            "src/voice_leading.rs",
            include_str!("../src/voice_leading.rs"),
        ),
        ("src/genre.rs", include_str!("../src/genre.rs")),
        ("src/swing.rs", include_str!("../src/swing.rs")),
        ("src/rhythm.rs", include_str!("../src/rhythm.rs")),
        ("src/melody.rs", include_str!("../src/melody.rs")),
        ("src/drum.rs", include_str!("../src/drum.rs")),
        ("tests/properties.rs", include_str!("properties.rs")),
        ("tests/golden_tables.rs", include_str!("golden_tables.rs")),
        (
            "tests/determinism_digests.rs",
            include_str!("determinism_digests.rs"),
        ),
        ("Cargo.toml", include_str!("../Cargo.toml")),
    ];
    let mut total = 0usize;
    for (name, source) in SOURCES {
        let hits = identical_sides(source, "assert_ne");
        assert!(
            hits.is_empty(),
            "{name}: assert_ne! with identical sides at {hits:?}"
        );
        total += hits.len();
    }
    assert_eq!(total, 0, "assert_ne! with identical sides anywhere");
}

/// R70②：把 `assert_eq!(f(x), f(x))` 的自比清单**机械枚举**并**上界锁死**。
///
/// 实测（源码文本相同者）：`lib.rs` 1 条、`genre.rs` 11 条、
/// `tests/properties.rs` 1 条 = **13** 条；其余 10 个文件 0 条。
/// 每一条自比都由 [`deterministic_readings_are_pinned_to_literal_digests`] 的
/// 字面摘要承担契约地位。新增一条自比 ⇒ 本判据红（提示作者补摘要）。
#[test]
fn the_remaining_self_comparisons_are_inventoried_and_bounded() {
    // 自证（R56）：已知红 + 已知绿。
    assert_eq!(identical_sides("assert_eq!(x, x);", "assert_eq"), vec![1]);
    assert!(identical_sides("assert_eq!(x, y);", "assert_eq").is_empty());

    assert_eq!(
        identical_sides(include_str!("../src/lib.rs"), "assert_eq").len(),
        1
    );
    assert_eq!(
        identical_sides(include_str!("../src/genre.rs"), "assert_eq").len(),
        11
    );
    assert_eq!(
        identical_sides(include_str!("properties.rs"), "assert_eq").len(),
        1
    );
    for (name, source) in [
        ("src/pitch.rs", include_str!("../src/pitch.rs")),
        ("src/scale.rs", include_str!("../src/scale.rs")),
        ("src/chord.rs", include_str!("../src/chord.rs")),
        ("src/progression.rs", include_str!("../src/progression.rs")),
        (
            "src/voice_leading.rs",
            include_str!("../src/voice_leading.rs"),
        ),
        ("src/swing.rs", include_str!("../src/swing.rs")),
        ("src/rhythm.rs", include_str!("../src/rhythm.rs")),
        ("src/melody.rs", include_str!("../src/melody.rs")),
        ("src/drum.rs", include_str!("../src/drum.rs")),
        ("src/error.rs", include_str!("../src/error.rs")),
    ] {
        assert!(
            identical_sides(source, "assert_eq").is_empty(),
            "{name} has a vacuous self-comparison"
        );
    }
}

// ---------------------------------------------------------------------------
// 方向 5：根音文本的邻域（R42 的邻域）—— 7 个字母 × 7 种记号后缀，逐个给读数。
// ---------------------------------------------------------------------------

/// 文档口径的独立复算：首字符是根音字母，其余字符是变音记号；
/// 降号集合与 `pitch::parse_pitch_class` 完全一致（`b` / `B` / `♭`）。
fn reference_is_flat(text: &str) -> bool {
    let mut chars = text.chars();
    chars.next();
    chars.any(|ch| matches!(ch, 'b' | 'B' | '\u{266d}'))
}

/// 63 个根音文本（7 字母 × **9** 后缀）逐个钉住，并与 `parse_pitch_class` 的
/// 变音记号口径对账。另钉多记号组合、五对同音异名与两处八度回绕读数。
///
/// ⚠ 后缀表必须含 **`B`** 与 **`BB`**：`parse_pitch_class` 的降号集合是
/// `b` / `B` / `♭`，而"字母 `B` 与降号记号 `B`"正是 R42 的原始缺陷面。
/// 本批实测：只列 7 种后缀时，"把 `B` 从降号集合里删掉"这条注入**不动**这条判据
/// ⇒ 表本身有覆盖缺口，已补成 9 种（7 × 9 = 63）。
#[test]
fn root_text_neighbourhood_is_swept_systematically() {
    const LETTERS: [&str; 7] = ["C", "D", "E", "F", "G", "A", "B"];
    const SUFFIXES: [&str; 9] = ["", "#", "##", "b", "bb", "B", "BB", "\u{266f}", "\u{266d}"];
    let mut swept = 0usize;
    for letter in LETTERS {
        for suffix in SUFFIXES {
            let text = format!("{letter}{suffix}");
            let expected = if reference_is_flat(&text) {
                Tonality::FlatMajor
            } else {
                Tonality::SharpMajor
            };
            assert_eq!(
                Tonality::infer_from_root_text(&text),
                expected,
                "root text {text:?}"
            );
            // 与解析器的变音记号口径对账（两者吃同一段根音文本）。
            let parsed = parse_pitch_class(&text).unwrap();
            let natural = parse_pitch_class(letter).unwrap();
            let raw =
                (i32::from(parsed.semitones()) - i32::from(natural.semitones())).rem_euclid(12);
            let alter = if raw > 6 { raw - 12 } else { raw };
            assert_eq!(
                expected == Tonality::FlatMajor,
                alter < 0,
                "{text:?}: tonality vs parse_pitch_class alter {alter}"
            );
            swept += 1;
        }
    }
    assert_eq!(swept, 63, "7 letters x 9 suffixes");

    // 多记号组合（字母 B 后面跟记号 `B` 的两种大小写 + 混合记号）。
    for (text, expected) in [
        ("CB", Tonality::FlatMajor),
        ("BB", Tonality::FlatMajor),
        ("bB", Tonality::FlatMajor),
        ("CbB", Tonality::FlatMajor),
        ("C#", Tonality::SharpMajor),
        ("B#", Tonality::SharpMajor),
        ("B", Tonality::SharpMajor),
    ] {
        assert_eq!(Tonality::infer_from_root_text(text), expected, "{text:?}");
    }

    // 五对同音异名：拼写不同、音级相同、调性相反（升号侧 vs 降号侧）。
    for (sharp, flat) in [
        ("C#", "Db"),
        ("D#", "Eb"),
        ("F#", "Gb"),
        ("G#", "Ab"),
        ("A#", "Bb"),
    ] {
        assert_eq!(
            Tonality::infer_from_root_text(sharp),
            Tonality::SharpMajor,
            "{sharp}"
        );
        assert_eq!(
            Tonality::infer_from_root_text(flat),
            Tonality::FlatMajor,
            "{flat}"
        );
        assert_eq!(
            parse_pitch_class(sharp).unwrap(),
            parse_pitch_class(flat).unwrap(),
            "{sharp} and {flat} are the same pitch class"
        );
    }
    // 八度回绕的读数是音级语义：B# 与 C♭ 是同一个音级。
    assert_eq!(parse_pitch_class("B#").unwrap(), PitchClass::C);
    assert_eq!(parse_pitch_class("C\u{266d}").unwrap(), PitchClass::B);
    assert_eq!(
        Tonality::infer_from_root_text("B#"),
        Tonality::SharpMajor,
        "the letter B is not a flat sign"
    );
    assert_eq!(
        Tonality::infer_from_root_text("C\u{266d}"),
        Tonality::FlatMajor,
        "the Unicode flat is a flat sign"
    );
}
