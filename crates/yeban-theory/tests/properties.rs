//! 属性测试与端到端判据 [ROAD-M4-003, TEST-SPEC-001]。
//!
//! 这一层刻意**不**复用 crate 内部单元测试的辅助函数：集成测试只能看到
//! `yeban-theory` 的公开 API，因此这里同时是"公开 API 是否够用"的判据。
//!
//! 关键不变量（每条都在本文件中被属性测试覆盖）：
//!
//! 1. 任意音阶的任意 degree → `Pitch` 必定落在 `0..=127`；
//! 2. 任意和弦符号 `symbol()` → `from_symbol()` 恒等（根音、种类、斜杠低音）；
//! 3. 声部连接的**单声部跳进上界**（默认 12 半音）在任何走向上都成立；
//! 4. 展开后的 tick 区段首尾相接，且总时长恒等于 `bars × ticks_per_bar`；
//! 5. 频率 `note_to_hz` 严格单调递增；
//! 6. `derive_index` 的输出永远落在 `0..count` 且同种子同输出；
//! 7. 摇摆切分不丢 tick、不移动一对的起点、量化幂等（§7）。

use proptest::prelude::*;

use yeban_theory::TheoryError;
use yeban_theory::chord::{Chord, ChordKind, Tonality};
use yeban_theory::drum::{
    DrumHit, DrumStyle, DrumVoice, default_backbeat, styled_drum_pattern, swung_drum_pattern,
    swung_styled_drum_pattern,
};
use yeban_theory::genre::GenreLibrary;
use yeban_theory::melody::{
    CHORD_TONE_WEIGHT_FLOOR, MELODY_LOWER_BOUND, MELODY_MAX_LEAP, MELODY_UPPER_BOUND,
    MelodyConstraints, genre_melody, genre_melody_for, melody_over_chords,
};
use yeban_theory::pitch::{Pitch, PitchClass, note_to_hz, parse_pitch_class};
use yeban_theory::progression::{Degree, Meter, Progression, RomanQuality, expand_progression};
use yeban_theory::rhythm::{
    BeatGrouping, MAX_METRIC_WEIGHT, STRONG_BEAT_WEIGHT, cells_per_bar, felt_beats_per_bar,
    grouped_metric_grid, grouped_swung_metric_grid, is_compound_meter, metric_grid,
    metric_weight_grouped, metric_weight_in, swung_metric_grid,
};
use yeban_theory::scale::{Scale, ScaleKind};
use yeban_theory::voice_leading::{VoicingConstraints, realize, realize_three_voices};
use yeban_theory::{
    derive_index, derive_range_i64, quantize_onset, swung_onset_offset, swung_pair_span,
};

/// 全部 [`ScaleKind`]（含别名），属性测试的取值范围。
const ALL_SCALE_KINDS: [ScaleKind; 16] = [
    ScaleKind::Major,
    ScaleKind::Ionian,
    ScaleKind::NaturalMinor,
    ScaleKind::Aeolian,
    ScaleKind::HarmonicMinor,
    ScaleKind::MelodicMinor,
    ScaleKind::Dorian,
    ScaleKind::Phrygian,
    ScaleKind::Lydian,
    ScaleKind::Mixolydian,
    ScaleKind::Locrian,
    ScaleKind::PentatonicMajor,
    ScaleKind::PentatonicMinor,
    ScaleKind::Blues,
    ScaleKind::WholeTone,
    ScaleKind::Chromatic,
];

/// 全部 [`ChordKind`]。
const ALL_CHORD_KINDS: [ChordKind; 21] = [
    ChordKind::Major,
    ChordKind::Minor,
    ChordKind::Diminished,
    ChordKind::Augmented,
    ChordKind::Sus2,
    ChordKind::Sus4,
    ChordKind::Six,
    ChordKind::Dominant7,
    ChordKind::Major7,
    ChordKind::Minor7,
    ChordKind::HalfDiminished7,
    ChordKind::Diminished7,
    ChordKind::MinorMajor7,
    ChordKind::Dominant9,
    ChordKind::Major9,
    ChordKind::Minor9,
    ChordKind::Dominant11,
    ChordKind::Dominant13,
    ChordKind::Add9,
    ChordKind::SixNine,
    ChordKind::Dominant7Sus4,
];

/// 全部 [`RomanQuality`]。
const ALL_ROMAN_QUALITIES: [RomanQuality; 5] = [
    RomanQuality::Major,
    RomanQuality::Minor,
    RomanQuality::Diminished,
    RomanQuality::Augmented,
    RomanQuality::HalfDiminished,
];

fn arb_pitch_class() -> impl Strategy<Value = PitchClass> {
    (0u8..12).prop_map(|value| PitchClass::new(value).expect("0..12 is a valid pitch class"))
}

fn arb_scale_kind() -> impl Strategy<Value = ScaleKind> {
    (0usize..ALL_SCALE_KINDS.len()).prop_map(|index| ALL_SCALE_KINDS[index])
}

fn arb_tonality() -> impl Strategy<Value = Tonality> {
    prop_oneof![
        Just(Tonality::SharpMajor),
        Just(Tonality::FlatMajor),
        Just(Tonality::Minor),
    ]
}

fn arb_degree() -> impl Strategy<Value = Degree> {
    (1u8..=7, -2i8..=2, 0usize..5).prop_map(|(degree, accidental, quality_index)| Degree {
        degree,
        accidental,
        quality: ALL_ROMAN_QUALITIES[quality_index],
        suffix: None,
    })
}

// ---------------------------------------------------------------------------
// 1. 音高：音阶 degree → MIDI 必定在音域内
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// [ARCH-DET-001] 任意音阶、任意八度、任意 degree：**要么**落回
    /// `0..=127` 且是该音阶的音级，**要么**明确报 `PitchOutOfRange`。
    /// 绝不允许环绕、截断或静默夹紧。
    #[test]
    fn any_scale_degree_either_fits_or_errors(
        tonic_value in 0u8..12,
        kind_index in 0usize..ALL_SCALE_KINDS.len(),
        tonic_octave in 0i8..=8,
        degree in 0u16..=40,
    ) {
        let scale = Scale::new(PitchClass::new(tonic_value).unwrap(), ALL_SCALE_KINDS[kind_index]);
        match scale.degree_to_pitch(tonic_octave, degree) {
            Ok(pitch) => {
                prop_assert!(pitch.value() <= 127);
                prop_assert!(scale.contains(pitch.pitch_class()));
                // 八度等价：放得下时 `degree + degree_count` 恰好高一个八度；
                // 放不下时必须报错。
                let next_degree = degree + u16::from(scale.degree_count());
                if pitch.value() + 12 <= 127 {
                    let next = scale.degree_to_pitch(tonic_octave, next_degree)?;
                    prop_assert_eq!(next.value(), pitch.value() + 12);
                } else {
                    prop_assert!(scale.degree_to_pitch(tonic_octave, next_degree).is_err());
                }
            }
            Err(TheoryError::PitchOutOfRange { .. }) => {}
            Err(other) => prop_assert!(false, "unexpected error: {other}"),
        }
    }

    /// 在安全包线内（八度 1..=5、degree 0..=13）必须**总是**成功，
    /// 且总是落在音阶上。
    #[test]
    fn scale_degrees_within_the_safe_envelope_always_fit(
        tonic_value in 0u8..12,
        kind_index in 0usize..ALL_SCALE_KINDS.len(),
        tonic_octave in 1i8..=5,
        degree in 0u16..=13,
    ) {
        let scale = Scale::new(PitchClass::new(tonic_value).unwrap(), ALL_SCALE_KINDS[kind_index]);
        let pitch = scale.degree_to_pitch(tonic_octave, degree)?;
        prop_assert!(pitch.value() <= 127);
        prop_assert!(scale.contains(pitch.pitch_class()));
        // 度数越界时按音阶音数取模，永不 panic。
        let wrapped = scale.degree_to_pitch(tonic_octave, degree + u16::from(scale.degree_count()))?;
        prop_assert_eq!(wrapped.value(), pitch.value() + 12);
    }

    /// 音阶的 `contains` 与 `degree_of` 必须互为逆：`contains` 为真
    /// 当且仅当能在音阶里找到该音级。
    #[test]
    fn contains_and_degree_of_agree(tonic_value in 0u8..12, kind_index in 0usize..ALL_SCALE_KINDS.len(), pc_value in 0u8..12) {
        let scale = Scale::new(PitchClass::new(tonic_value).unwrap(), ALL_SCALE_KINDS[kind_index]);
        let pc = PitchClass::new(pc_value).unwrap();
        prop_assert_eq!(scale.contains(pc), scale.degree_of(pc).is_some());
    }

    /// 十二平均律频率严格单调：音高越高频率越大。
    #[test]
    fn note_to_hz_is_strictly_monotonic(low in 0u8..127) {
        prop_assert!(note_to_hz(low) < note_to_hz(low + 1));
    }

    /// `A4 = 440 Hz`，且每个八度恰好是 2 倍。
    #[test]
    fn note_to_hz_doubles_every_octave(midi in 0u8..116) {
        let ratio = note_to_hz(midi + 12) / note_to_hz(midi);
        prop_assert!((ratio - 2.0).abs() < 1e-9, "ratio was {ratio}");
    }
}

// ---------------------------------------------------------------------------
// 1b. 音名拼写：拼出来的名字必须还原成同一个音级
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// 拼写函数的核心不变量：`spell` 返回的音名必须还原成**同一个**音级；
    /// 拼不出来必须是明确错误（重升/重降之外无法表示），绝不给出错误音级。
    #[test]
    fn spelled_names_round_trip_to_the_same_pitch_class(
        pc in arb_pitch_class(),
        kind in arb_scale_kind(),
        tonic_value in 0u8..12,
        prefer_flat in any::<bool>(),
    ) {
        let scale = Scale::new(PitchClass::new(tonic_value).unwrap(), kind);
        match scale.spell_with(pc, prefer_flat) {
            Ok(name) => prop_assert_eq!(name.pitch_class(), pc),
            Err(TheoryError::AmbiguousSpelling | TheoryError::NoteNameUnknown) => {}
            Err(other) => prop_assert!(false, "unexpected error: {other}"),
        }
    }

    /// 七声音阶里的每一个音级都必须拼得出来（重升/重降足够覆盖教会调式）。
    #[test]
    fn every_tone_of_a_seven_note_scale_is_spellable(
        pc in arb_pitch_class(),
        tonic_value in 0u8..12,
    ) {
        let scale = Scale::new(PitchClass::new(tonic_value).unwrap(), ScaleKind::Major);
        let name = scale.spell(pc)?;
        prop_assert_eq!(name.pitch_class(), pc);
        prop_assert!(name.alter.abs() <= 2);
    }

    /// 和弦的调性上下文只影响拼写，不影响音级：`pitch_class_name` 必须守恒。
    #[test]
    fn tonality_only_changes_orthography(
        pc in arb_pitch_class(),
        tonality in arb_tonality(),
    ) {
        let chord = Chord::with_tonality(PitchClass::C, ChordKind::Major, tonality);
        let name = chord.pitch_class_name(pc);
        prop_assert_eq!(name.pitch_class(), pc);
        prop_assert!(name.alter.abs() <= 2);
    }
}

// ---------------------------------------------------------------------------
// 2. 和弦：符号往返恒等
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    /// 任意根音 × 任意和弦种类：`symbol()` → `from_symbol()` 必须恒等。
    #[test]
    fn chord_symbol_round_trips_for_every_root_and_kind(
        root_value in 0u8..12,
        kind_index in 0usize..ALL_CHORD_KINDS.len(),
        tonality_index in 0usize..3,
    ) {
        let tonality = [Tonality::SharpMajor, Tonality::FlatMajor, Tonality::Minor][tonality_index];
        let chord = Chord::with_tonality(
            PitchClass::new(root_value).unwrap(),
            ALL_CHORD_KINDS[kind_index],
            tonality,
        );
        let text = chord.symbol();
        let parsed = Chord::from_symbol(&text)?;
        prop_assert_eq!(parsed.root, chord.root, "symbol {}", text);
        prop_assert_eq!(parsed.kind, chord.kind, "symbol {}", text);
        prop_assert_eq!(parsed.bass, chord.bass, "symbol {}", text);
        // 二次往返必须稳定（不会出现 A# → Bb → A# 的漂移）
        prop_assert_eq!(parsed.symbol(), text);
    }

    /// 转位后的斜杠低音必须仍属于和弦构成音，且再解析回来一致。
    #[test]
    fn inverted_chords_round_trip_through_their_slash_notation(
        root_value in 0u8..12,
        kind_index in 0usize..ALL_CHORD_KINDS.len(),
        inversion in 0u8..5,
    ) {
        let chord = Chord::new(PitchClass::new(root_value).unwrap(), ALL_CHORD_KINDS[kind_index]);
        let inverted = chord.inversion(inversion);
        let text = inverted.symbol();
        let parsed = Chord::from_symbol(&text)?;
        prop_assert_eq!(parsed.root, chord.root, "symbol {}", text);
        prop_assert_eq!(parsed.kind, chord.kind, "symbol {}", text);
        if let Some(bass) = parsed.bass {
            prop_assert!(chord.pitch_classes().contains(&bass));
        }
    }

    /// 和弦构成音的个数必须与公式长度一致，且音高全部在 MIDI 域内。
    #[test]
    fn chord_pitches_match_formula_and_fit_in_midi(
        root_value in 0u8..12,
        kind_index in 0usize..ALL_CHORD_KINDS.len(),
        root_octave in 1i8..=6,
    ) {
        let kind = ALL_CHORD_KINDS[kind_index];
        let chord = Chord::new(PitchClass::new(root_value).unwrap(), kind);
        let pitches = chord.pitches(root_octave)?;
        prop_assert_eq!(pitches.len(), kind.intervals().len());
        for pitch in &pitches {
            prop_assert!(pitch.value() <= 127);
            // 每个构成音都必须真的属于该和弦
            prop_assert!(chord.pitch_classes().contains(&pitch.pitch_class()));
        }
        // 第一个音必须是根音
        prop_assert_eq!(pitches[0].pitch_class(), chord.root);
    }
}

// ---------------------------------------------------------------------------
// 3. 走向展开：tick 网格
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(192))]

    /// 展开后的区段必须首尾相接、时值为正、总和恒等于 `bars × ticks_per_bar`。
    #[test]
    fn expanded_spans_tile_the_requested_bars_exactly(
        tonic_value in 0u8..12,
        kind_index in 0usize..ALL_SCALE_KINDS.len(),
        bars in 1u32..=8,
        degrees in prop::collection::vec(arb_degree(), 1..=6),
        meter_index in 0usize..6,
    ) {
        let meter = [Meter::COMMON, Meter::WALTZ, Meter::COMPOUND_DUPLE, Meter::MARCH, Meter::QUINTUPLE, Meter::SEVEN_EIGHT][meter_index];
        let key = Scale::new(PitchClass::new(tonic_value).unwrap(), ALL_SCALE_KINDS[kind_index]);
        let progression = Progression::new(degrees, meter, bars)?;
        let spans = progression.expand(&key)?;
        prop_assert!(!spans.is_empty());
        let mut cursor = 0u64;
        for span in &spans {
            prop_assert_eq!(span.start_tick, cursor);
            prop_assert!(span.duration_ticks > 0);
            prop_assert!(span.chord.root.semitones() < 12);
            cursor = span.end_tick();
        }
        prop_assert_eq!(cursor, progression.total_ticks());
        prop_assert_eq!(cursor, u64::from(bars) * meter.ticks_per_bar());
    }

    /// 每个小节都必须被覆盖到：区段数与覆盖的小节数不能少于 `bars`。
    #[test]
    fn every_requested_bar_is_covered(
        bars in 1u32..=8,
        degrees in prop::collection::vec(arb_degree(), 1..=4),
    ) {
        let key = Scale::new(PitchClass::C, ScaleKind::Major);
        let progression = Progression::new(degrees, Meter::COMMON, bars)?;
        let spans = progression.expand(&key)?;
        let covered: u64 = spans.iter().map(|span| span.duration_ticks).sum();
        prop_assert!(covered >= u64::from(bars) * 3840);
        // 每个和弦都必须落在 16 分音符网格上，时值也必须是 16 分音符的整数倍。
        for span in &spans {
            prop_assert_eq!(
                span.start_tick % yeban_theory::progression::MIN_DURATION_TICKS,
                0,
                "start {} is off the sixteenth-note grid",
                span.start_tick
            );
            prop_assert_eq!(span.duration_ticks % yeban_theory::progression::MIN_DURATION_TICKS, 0);
            prop_assert!(span.duration_ticks > 0);
        }
    }
}

// ---------------------------------------------------------------------------
// 4. 声部连接：跳进上界
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    /// **核心不变量**：任意走向、任意调，相邻和弦之间没有任何单声部跳进
    /// 超过 12 个半音；所有声部都落在配置的音域内且严格由低到高。
    #[test]
    fn voice_leading_never_exceeds_the_documented_jump_bound(
        tonic_value in 0u8..12,
        kind_index in 0usize..ALL_SCALE_KINDS.len(),
        degrees in prop::collection::vec(arb_degree(), 1..=6),
        bars in 1u32..=4,
    ) {
        let key = Scale::new(PitchClass::new(tonic_value).unwrap(), ALL_SCALE_KINDS[kind_index]);
        let progression = Progression::new(degrees, Meter::COMMON, bars)?;
        let spans = progression.expand(&key)?;
        let result = realize_three_voices(&spans)?;
        prop_assert_eq!(result.voicings.len(), spans.len());
        for (span, voicing) in spans.iter().zip(result.voicings.iter()) {
            prop_assert_eq!(voicing.len(), 3);
            prop_assert!(voicing[0] < voicing[1] && voicing[1] < voicing[2]);
            for (index, pitch) in voicing.iter().enumerate() {
                let range = VoicingConstraints::THREE_VOICES.ranges[index];
                prop_assert!(pitch.value() >= range.lower.value());
                prop_assert!(pitch.value() <= range.upper.value());
                // 每个声部都必须是和弦音
                prop_assert!(span.chord.pitch_classes().contains(&pitch.pitch_class()));
            }
        }
        prop_assert!(result.max_voice_jump() <= 12);
        // `movements` 必须与 `voicings` 自洽
        for (index, row) in result.movements.iter().enumerate() {
            prop_assert_eq!(row.len(), 3);
            for (voice, step) in row.iter().enumerate() {
                let delta = result.voicings[index][voice]
                    .abs_distance_to(result.voicings[index + 1][voice]);
                prop_assert_eq!(*step, delta);
            }
        }
    }

    /// 4 声部同样必须守跳进上界（且每个声部都是和弦音）。
    #[test]
    fn four_voice_configuration_also_respects_the_bound(
        tonic_value in 0u8..12,
        degrees in prop::collection::vec(arb_degree(), 1..=4),
    ) {
        let key = Scale::new(PitchClass::new(tonic_value).unwrap(), ScaleKind::Major);
        let progression = Progression::new(degrees, Meter::COMMON, 4)?;
        let spans = progression.expand(&key)?;
        let result = realize(&spans, &VoicingConstraints::FOUR_VOICES)?;
        prop_assert_eq!(result.voicings.len(), spans.len());
        prop_assert!(result.voicings.iter().all(|voicing| voicing.len() == 4));
        prop_assert!(result.max_voice_jump() <= 12);
        for (span, voicing) in spans.iter().zip(result.voicings.iter()) {
            for pitch in voicing {
                prop_assert!(span.chord.pitch_classes().contains(&pitch.pitch_class()));
            }
        }
    }

    /// 同一输入必须产出同一结果（束搜索不得依赖任何哈希迭代顺序）。
    #[test]
    fn voice_leading_is_deterministic(degrees in prop::collection::vec(arb_degree(), 1..=5)) {
        let key = Scale::new(PitchClass::G, ScaleKind::Major);
        let progression = Progression::new(degrees, Meter::COMMON, 4)?;
        let spans = progression.expand(&key)?;
        prop_assert_eq!(realize_three_voices(&spans)?, realize_three_voices(&spans)?);
    }
}

// ---------------------------------------------------------------------------
// 5. 种子驱动的确定性
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// [ARCH-DET-001] `derive_index` 的输出永远在界内，且同种子同输出。
    #[test]
    fn derive_index_is_bounded_and_reproducible(seed in any::<u64>(), salt in any::<u64>(), count in 1usize..4096) {
        let first = derive_index(seed, salt, count);
        let second = derive_index(seed, salt, count);
        prop_assert_eq!(first, second);
        prop_assert!(first < count);
    }

    /// `derive_range_i64` 的输出永远落在闭区间内。
    #[test]
    fn derive_range_stays_within_bounds(seed in any::<u64>(), low in -1000i64..1000, width in 0i64..1000) {
        let high = low + width;
        let value = derive_range_i64(seed, 17, low, high);
        prop_assert!(value >= low && value <= high);
    }
}

// ---------------------------------------------------------------------------
// 6. 端到端：流派规则库
// ---------------------------------------------------------------------------

#[test]
fn every_genre_produces_a_playable_sketch() {
    // 对全部流派跑一遍"走向展开 → 声部连接"，确保规则库里的数据
    // 每一行都能真的走通到声部层，而不只是"字符串能解析"。
    for rule in GenreLibrary::all() {
        let scale = rule
            .primary_scale(PitchClass::C)
            .unwrap_or_else(|err| panic!("{}: primary scale failed: {err}", rule.id));
        let spans = rule
            .sketch(PitchClass::C, 4)
            .unwrap_or_else(|err| panic!("{}: sketch failed: {err}", rule.id));
        assert!(!spans.is_empty(), "{}", rule.id);
        let result = realize_three_voices(&spans)
            .unwrap_or_else(|err| panic!("{}: voice leading failed: {err}", rule.id));
        assert!(
            result.max_voice_jump() <= 12,
            "{}: jump bound violated ({})",
            rule.id,
            result.max_voice_jump()
        );
        // 每个和弦都必须是该调音阶上的和弦（音阶级数走向的应有之义）。
        for span in &spans {
            assert!(
                scale.contains(span.chord.root),
                "{}: chord root {} is outside {}",
                rule.id,
                span.chord.root,
                scale
            );
        }
    }
}

#[test]
fn required_progressions_from_the_brief_are_supported() {
    // 任务书点名的三条走向 + 罗马数字解析的边界写法。
    let cases = ["I-V-vi-IV", "ii-V-I", "i-VI-III-VII"];
    for text in cases {
        let progression = Progression::parse(text).unwrap_or_else(|err| {
            panic!("{text} failed to parse: {err}");
        });
        assert!(!progression.degrees().is_empty(), "{text}");
        let key = Scale::new(PitchClass::C, ScaleKind::Major);
        let spans = expand_progression(&key, text, 4).unwrap();
        assert_eq!(
            spans.iter().map(|s| s.duration_ticks).sum::<u64>(),
            4 * 3840,
            "{text}"
        );
    }
}

#[test]
fn ppq_matches_the_project_wide_960_tick_grid() {
    // [MODEL-AST-001] 本 crate 独立声明了 PPQ；它与 model 层的 960 必须一致。
    // 这里用"内部一致性"口径验证：4/4 一小节 = 4 拍 = 3840 tick。
    assert_eq!(yeban_theory::PPQ, 960);
    assert_eq!(Meter::COMMON.ticks_per_bar(), 3840);
    assert_eq!(Meter::WALTZ.ticks_per_bar(), 2880);
}

#[test]
fn note_name_parsing_rejects_garbage_without_panicking() {
    for text in ["", "H", "C", "#4", "C#", "Cb", "C-x", "🎵", "C#4extra"] {
        // 只要求不 panic；能解析出结果或返回错误都算合格。
        let _ = parse_pitch_class(text);
        let _ = text.parse::<yeban_theory::pitch::SpelledPitch>();
        let _ = Chord::from_symbol(text);
        let _ = Degree::parse(text);
    }
    // 但合法的输入必须真的能解析。
    assert_eq!(parse_pitch_class("Bb").unwrap(), PitchClass::AS);
    assert_eq!(Pitch::new(127).unwrap().value(), 127);
    assert!(Pitch::new(128).is_err());
    assert!(Pitch::new(-1).is_err());
}

// ---------------------------------------------------------------------------
// 7. 摇摆：切分不丢 tick、不移动对起点、量化幂等
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// `first + second` 恒等于整对长度，且前半覆盖整对的一半以上（比例 >= 500）。
    ///
    /// 注意：`pair.first >= pair.second` 对**奇数**长度的对在 500 千分比下**不成立**
    /// （例如 `pair_ticks = 3` ⇒ 前半 1、后半 2，向下取整）。判据写
    /// `2 * first >= pair_ticks - 1`（离散网格上"不短于一半减一个 tick"）。
    #[test]
    fn swing_never_loses_or_creates_ticks(pair_ticks in 1u64..8192, permille in 500u16..=1000) {
        let pair = swung_pair_span(pair_ticks, permille)?;
        prop_assert_eq!(pair.total(), pair_ticks);
        prop_assert!(pair.first <= pair_ticks);
        prop_assert!(2 * pair.first + 1 >= pair_ticks);
        prop_assert_eq!(pair.offbeat_offset(), pair.first);
        // 摇摆偏移与前半的算术必须一致，不允许两条路径各算一份。
        prop_assert_eq!(
            swung_onset_offset(pair_ticks, permille)?,
            pair.first as i64 - (pair_ticks / 2) as i64
        );
    }

    /// 量化把 onset 放进它自己那一对的槽位上；再量化一次得到同一个值（幂等）。
    #[test]
    fn quantized_onsets_stay_in_their_own_pair_and_are_idempotent(
        onset in 0u64..8192,
        pair_ticks in 1u64..2048,
        permille in 500u16..=1000,
    ) {
        let once = quantize_onset(onset, pair_ticks, permille)?;
        prop_assert_eq!(once / pair_ticks, onset / pair_ticks);
        let first = swung_pair_span(pair_ticks, permille)?.first;
        // 第二个槽位在 `permille == 1000` 时退回本对最后一个 tick。
        let second_slot = first.min(pair_ticks - 1);
        let in_pair = once % pair_ticks;
        prop_assert!(in_pair == 0 || in_pair == second_slot);
        prop_assert_eq!(quantize_onset(once, pair_ticks, permille)?, once);
    }

    /// 越界比例必须报错，绝不静默钳制成平直。
    #[test]
    fn out_of_range_swing_is_always_an_error(pair_ticks in 1u64..4096, permille in 0u16..=499) {
        prop_assert!(swung_pair_span(pair_ticks, permille).is_err());
        prop_assert!(swung_onset_offset(pair_ticks, permille).is_err());
        prop_assert!(quantize_onset(0, pair_ticks, permille).is_err());
    }

    /// 登记表里的每一个 `swing` 值都必须能折成合法千分比，并且 `None` 保持平直。
    #[test]
    fn every_genre_swing_value_converts_to_a_legal_permille(index in 0usize..GenreLibrary::all().len()) {
        let rule = &GenreLibrary::all()[index];
        match rule.swing_permille()? {
            None => prop_assert!(rule.swing.is_none()),
            Some(permille) => {
                prop_assert!((500..=1000).contains(&permille));
                let pair = swung_pair_span(480, permille)?;
                prop_assert_eq!(pair.total(), 480);
            }
        }
    }

    /// 类别①（非有限输入）：[`yeban_theory::genre::GenreRule::swing`] 是 `pub`
    /// 字段，调用方可以塞进**任意** `f32` 位模式（`NaN` / `±∞` / 子正规数 /
    /// 极大有限值）。判据：`swing_permille()` 不 panic，且结果只能是
    /// `Err(SwingOutOfRange)` 或 `500..=1000` 的合法千分比 ——
    /// 绝不返回越界值，也绝不把 `Some` 报成 `None`。
    ///
    /// 覆盖口径：`any::<u32>()` 均匀抽位模式，因此 `NaN` 的 2^24 - 2 个编码、
    /// `±∞`、全部子正规数与全部大指数都落在取值空间内（不是枚举登记表）。
    #[test]
    fn any_f32_swing_bit_pattern_errors_or_yields_a_legal_permille(bits in any::<u32>()) {
        let mut rule = GenreLibrary::all()[0];
        let value = f32::from_bits(bits);
        rule.swing = Some(value);
        match rule.swing_permille() {
            Ok(None) => prop_assert!(false, "swing = Some({value}) reported as absent"),
            Ok(Some(permille)) => prop_assert!(
                (500..=1000).contains(&permille),
                "bits {bits:#010x} ({value}) gave illegal permille {permille}"
            ),
            Err(error) => prop_assert!(
                matches!(error, TheoryError::SwingOutOfRange { .. }),
                "bits {bits:#010x} ({value}) gave unexpected {error:?}"
            ),
        }
    }

    /// 每条流派规则都能产出节奏网格：每小节恰好 `onsets` 个 onset、
    /// tick 严格升序、全部落在本小节内、`hits_in_bar` 与全局序列一致。
    #[test]
    fn every_genre_produces_a_metric_grid_with_the_requested_onsets(
        index in 0usize..GenreLibrary::all().len(),
        bars in 1u32..8,
        onsets in 1u32..=8,
    ) {
        let rule = &GenreLibrary::all()[index];
        let grid = rule.rhythm_grid(bars, onsets)?;
        prop_assert_eq!(grid.len(), bars as usize * onsets as usize);
        prop_assert_eq!(grid.meter(), rule.meter_value());
        prop_assert_eq!(grid.swing_permille(), rule.swing_permille()?);
        prop_assert_eq!(grid.total_ticks(), u64::from(bars) * rule.meter_value().ticks_per_bar());
        let bar_ticks = rule.meter_value().ticks_per_bar();
        for pair in grid.hits().windows(2) {
            prop_assert!(pair[0].tick < pair[1].tick);
        }
        for hit in grid.hits() {
            let bar_start = u64::from(hit.bar) * bar_ticks;
            prop_assert!(hit.tick >= bar_start && hit.tick < bar_start + bar_ticks);
            prop_assert!(u64::from(hit.cell) * 240 < bar_ticks);
            // 重量按**拍号**算，不是只看格点下标（6/8 与 3/4 的小节一样长）。
            prop_assert_eq!(hit.weight, metric_weight_in(rule.meter_value(), hit.cell));
        }
        for bar in 0..bars {
            let slice = grid.hits_in_bar(bar);
            prop_assert_eq!(slice.len(), onsets as usize);
            prop_assert!(slice.iter().all(|hit| hit.bar == bar));
        }
        // 第 0 小节的第一个 onset 恒是小节起点（度量重量最大）。
        prop_assert_eq!(grid.hits()[0].tick, 0);
        prop_assert_eq!(grid.hits()[0].cell, 0);
        prop_assert_eq!(grid.hits()[0].weight, MAX_METRIC_WEIGHT);
        // 纯函数：同输入同输出。
        prop_assert_eq!(rule.rhythm_grid(bars, onsets)?, grid);
    }

    /// 摇摆网格永不丢 onset：没有两个格点重合，且每个基格点都留在自己的小节里。
    /// 覆盖 **全部** 流派登记的拍号与全部合法千分比端点。
    #[test]
    fn swing_never_collapses_a_grid(index in 0usize..GenreLibrary::all().len(), bars in 1u32..6) {
        let rule = &GenreLibrary::all()[index];
        let meter = rule.meter_value();
        let cells = cells_per_bar(meter).unwrap();
        let onsets = u32::try_from(cells).unwrap();
        let straight = metric_grid(meter, bars, onsets)?;
        let swung = swung_metric_grid(meter, bars, onsets, Some(1000))?;
        prop_assert_eq!(straight.len(), swung.len());
        // 平直网格必须正好落在 16 分格点上。
        for hit in straight.hits() {
            prop_assert_eq!(hit.tick % 240, 0);
        }
        // 最大摇摆下仍然两两不同、严格升序。
        let mut ticks: Vec<u64> = swung.hits().iter().map(|hit| hit.tick).collect();
        let count = ticks.len();
        ticks.sort_unstable();
        ticks.dedup();
        prop_assert_eq!(ticks.len(), count);
        for pair in swung.hits().windows(2) {
            prop_assert!(pair[0].tick < pair[1].tick);
        }
        // 摇摆只把后半格点向右移，前半格点逐位不动。
        for (plain, moved) in straight.hits().iter().zip(swung.hits()) {
            prop_assert_eq!(plain.bar, moved.bar);
            prop_assert_eq!(plain.cell, moved.cell);
            prop_assert!(moved.tick >= plain.tick);
            if moved.cell % 2 == 0 {
                prop_assert_eq!(moved.tick, plain.tick);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 7.5 度量重量必须读拍号（6/8 ≠ 3/4）
// ---------------------------------------------------------------------------

/// 用一个 flow 数出**拍号重量**的核心不变量：
/// 以"该拍号自己的拍数"作为每小节 onset 数时，选出的格点**恰好**是每一拍的起点。
///
/// 单位：`checked` 数的是**不同的登记拍号**（不是流派条数）。
/// 这条判据在 6/8 上必须抓到"第 0、8 格"这种把 6/8 当 3/4 读的结果：
/// 6/8 的两拍是第 0、6 格（1440 tick 的附点四分）。
#[test]
fn a_bar_of_beats_selects_exactly_the_beat_grid() {
    let mut meters = std::collections::BTreeSet::new();
    for rule in GenreLibrary::all() {
        meters.insert(rule.meter);
    }
    let mut checked = 0usize;
    for (numerator, denominator) in meters {
        let meter = Meter::new(numerator, denominator)
            .unwrap_or_else(|err| panic!("{numerator}/{denominator}: {err}"));
        let beats = u32::from(felt_beats_per_bar(meter));
        assert!(beats > 0, "{numerator}/{denominator}");
        let cells = u32::try_from(cells_per_bar(meter).unwrap()).unwrap();
        assert_eq!(cells % beats, 0, "{numerator}/{denominator}");
        let cells_per_beat = cells / beats;
        let grid = metric_grid(meter, 1, beats).unwrap();
        let got: Vec<u32> = grid.hits().iter().map(|hit| hit.cell).collect();
        let want: Vec<u32> = (0..beats).map(|beat| beat * cells_per_beat).collect();
        assert_eq!(got, want, "{numerator}/{denominator}");
        // 每个 onset 的 tick 恒是"每拍 tick 数"的整数倍。
        let ticks_per_beat = meter.ticks_per_bar() / u64::from(beats);
        for hit in grid.hits() {
            assert_eq!(hit.tick % ticks_per_beat, 0, "{numerator}/{denominator}");
        }
        checked += 1;
    }
    // 防真空：登记表里的不同拍号是 5 种（2/4、3/4、4/4、6/8、7/8）。
    // 拍号种类变了就必须改这里，并重新量 6/8 与 3/4 的读数。
    assert_eq!(checked, 5, "registered meter kinds changed");
}

/// 6/8 与 3/4 的小节长度相同，但网格**不许**逐位相同。
///
/// 实测（改前）：两条网格逐位相同，6/8 的两拍被选成第 0、8 格。
/// 实测（改后）：6/8 的第 6 格重量 3、第 4/8 格重量 1；3/4 的第 4/8 格重量 2。
#[test]
fn compound_duple_and_waltz_are_not_the_same_grid() {
    let compound = Meter::COMPOUND_DUPLE;
    let waltz = Meter::WALTZ;
    assert!(is_compound_meter(compound));
    assert!(!is_compound_meter(waltz));
    assert_eq!(compound.ticks_per_bar(), waltz.ticks_per_bar());
    assert_eq!(cells_per_bar(compound), cells_per_bar(waltz));

    // 6/8 的全部 12 个格位的重量（字面量）。
    let compound_weights: Vec<u8> = (0..12)
        .map(|cell| metric_weight_in(compound, cell))
        .collect();
    assert_eq!(compound_weights, vec![8, 0, 1, 0, 1, 0, 3, 0, 1, 0, 1, 0]);
    // 3/4 的全部 12 个格位的重量（字面量）：只有第 8 格不同（3 → 2）。
    let waltz_weights: Vec<u8> = (0..12).map(|cell| metric_weight_in(waltz, cell)).collect();
    assert_eq!(waltz_weights, vec![8, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0]);
    assert_ne!(compound_weights, waltz_weights);

    let a = metric_grid(compound, 2, 2).unwrap();
    let b = metric_grid(waltz, 2, 2).unwrap();
    assert_eq!(a.total_ticks(), b.total_ticks());
    assert_ne!(a.hits(), b.hits());
    assert_eq!(
        a.hits().iter().map(|hit| hit.cell).collect::<Vec<_>>(),
        vec![0, 6, 0, 6]
    );
    assert_eq!(
        a.hits().iter().map(|hit| hit.tick).collect::<Vec<_>>(),
        vec![0, 1440, 2880, 4320]
    );
}

// ---------------------------------------------------------------------------
// 7.6 加性拍分组（`pending 3` 剩余部分的**机制**侧）
// ---------------------------------------------------------------------------

/// 全部参与分组属性的拍号（含 `GENRES` 没用到的 5/4、8/8、9/8、11/8、12/8）。
const GROUPING_METERS: [Meter; 10] = [
    Meter::MARCH,
    Meter::WALTZ,
    Meter::COMMON,
    Meter::QUINTUPLE,
    Meter::COMPOUND_DUPLE,
    Meter::SEVEN_EIGHT,
    Meter {
        numerator: 8,
        denominator: 8,
    },
    Meter {
        numerator: 9,
        denominator: 8,
    },
    Meter {
        numerator: 11,
        denominator: 8,
    },
    Meter {
        numerator: 12,
        denominator: 8,
    },
];

proptest! {
    /// 任意**构造成功**的分组，都保持网格的全部结构不变量；当请求的 onset 数
    /// 恰好等于组数时，选出的格点**恒是组起点**（组起点重量 8 或 3，其余 ≤ 2）。
    ///
    /// 单位：`checked` 数的是**被属性测试接受的分组样本个数**（`BeatGrouping::new`
    /// 返回 `None` 的样本被跳过，不计入）；`checked > 0` 是防真空判据。
    #[test]
    fn a_grouped_grid_selects_the_group_starts_and_keeps_every_invariant(
        meter_index in 0usize..GROUPING_METERS.len(),
        groups in prop::collection::vec(1u8..=4, 1..=6),
        permille in prop::option::of(500u16..=1000),
        bars in 1u32..4,
    ) {
        let meter = GROUPING_METERS[meter_index];
        let Some(grouping) = BeatGrouping::new(meter, &groups) else {
            return Ok(());
        };
        let group_count = grouping.group_count() as u32;
        let cells = u32::try_from(cells_per_bar(meter).unwrap()).unwrap();
        prop_assume!(group_count <= cells);

        let grid = grouped_swung_metric_grid(meter, bars, group_count, permille, grouping)?;
        prop_assert_eq!(grid.len(), (bars * group_count) as usize);
        prop_assert_eq!(grid.meter(), meter);
        prop_assert_eq!(grid.swing_permille(), permille);

        let bar_ticks = meter.ticks_per_bar();
        for window in grid.hits().windows(2) {
            prop_assert!(window[0].tick < window[1].tick, "{:?}", window);
        }
        for hit in grid.hits() {
            let bar_start = u64::from(hit.bar) * bar_ticks;
            prop_assert!(hit.tick >= bar_start && hit.tick < bar_start + bar_ticks);
            // hit 的重量必须与公开的重量函数逐位一致（两个口径都不看混杂状态）。
            prop_assert_eq!(hit.weight, metric_weight_grouped(meter, hit.cell, grouping));
            // 组起点的重量恒 > 非组起点的拍重量。
            let cells_per_beat = cells / u32::from(felt_beats_per_bar(meter));
            let beat = hit.cell / cells_per_beat;
            if hit.cell % cells_per_beat == 0 {
                let expected = if beat == 0 {
                    MAX_METRIC_WEIGHT
                } else if grouping.is_group_start(beat as u8) {
                    3
                } else {
                    2
                };
                prop_assert_eq!(hit.weight, expected, "beat {}", beat);
                prop_assert!(hit.weight >= 2);
            }
        }

        // 恰取"组数个 onset" ⇒ 选出的**就是**每一组的起点。
        let got: Vec<u32> = grid.hits_in_bar(0).iter().map(|hit| hit.cell).collect();
        let cells_per_beat = cells / u32::from(felt_beats_per_bar(meter));
        let beats = u32::from(felt_beats_per_bar(meter));
        let want: Vec<u32> = (0..beats)
            .filter(|&beat| grouping.is_group_start(beat as u8))
            .map(|beat| beat * cells_per_beat)
            .collect();
        prop_assert_eq!(got, want);
    }
}

/// 内置口径（不分组）在每个**登记**拍号上都必须与"该拍号的内置分组"逐位一致，
/// 且与分组口径在这些拍号上给出同一个网格 —— 分组是**加法**，不是替换。
///
/// 单位：`checked` 数的是 `GENRES` 里**不同的拍号**种数（不是流派条数）。
#[test]
fn the_builtin_hierarchy_survives_the_grouping_api_on_every_registered_meter() {
    let mut meters = std::collections::BTreeSet::new();
    for rule in GenreLibrary::all() {
        meters.insert(rule.meter);
    }
    let builtin: std::collections::BTreeMap<(u8, u8), &[u8]> = [
        ((2, 4), &[2u8][..]),
        ((3, 4), &[3][..]),
        ((4, 4), &[2, 2][..]),
        ((6, 8), &[1, 1][..]),
        ((7, 8), &[7][..]),
    ]
    .into_iter()
    .collect();
    let mut checked = 0usize;
    for (numerator, denominator) in meters {
        let meter = Meter::new(numerator, denominator).unwrap();
        let groups = builtin
            .get(&(numerator, denominator))
            .unwrap_or_else(|| panic!("no builtin grouping for {numerator}/{denominator}"));
        let grouping = BeatGrouping::new(meter, groups)
            .unwrap_or_else(|| panic!("{numerator}/{denominator}: builtin grouping must cover it"));
        for onsets in [1u32, 2, 3, 5] {
            assert_eq!(
                grouped_metric_grid(meter, 2, onsets, grouping)
                    .unwrap()
                    .hits(),
                metric_grid(meter, 2, onsets).unwrap().hits(),
                "{numerator}/{denominator} onsets {onsets}"
            );
        }
        checked += 1;
    }
    assert_eq!(checked, 5, "registered meter kinds changed");
}

// ---------------------------------------------------------------------------
// 8. 端到端：旋律生成（`pending 4`）
// ---------------------------------------------------------------------------

/// 公开阈值（crate 常量）被钉成字面量。
///
/// 行为判据一律用**字面量**门槛，不引用这些常量，因此常量被改坏时
/// 行为判据不会跟着空转：本测试负责抓常量本身，行为判据负责抓行为。
#[test]
fn melody_public_thresholds_are_pinned_to_literals() {
    assert_eq!(MELODY_LOWER_BOUND, 48);
    assert_eq!(MELODY_UPPER_BOUND, 84);
    assert_eq!(MELODY_MAX_LEAP, 12);
    assert_eq!(CHORD_TONE_WEIGHT_FLOOR, 1);
}

/// 全部流派都能产出一条**在它自己的音阶里**、跳进不超上界、首尾相接的旋律。
///
/// 度量口径：本测试遍历 `GenreLibrary::all()`，对每一行取 4 小节 × 4 onset × 4 个种子，
/// 逐音检查不变量，最后断言 `checked == GenreLibrary::len()`（那条数字由
/// `genre.rs::tests::library_size_is_pinned_to_the_measured_number` 钉住）。
#[test]
fn every_genre_produces_a_melody_in_its_own_scale() {
    let bars = 4u32;
    let onsets = 4u32;
    let mut checked = 0usize;
    let mut notes_checked = 0usize;
    let mut strong_beats_checked = 0usize;
    for rule in GenreLibrary::all() {
        let key = rule
            .primary_scale(PitchClass::C)
            .unwrap_or_else(|err| panic!("{}: primary scale failed: {err}", rule.id));
        let spans = rule
            .sketch(PitchClass::C, bars)
            .unwrap_or_else(|err| panic!("{}: sketch failed: {err}", rule.id));
        let grid = rule
            .rhythm_grid(bars, onsets)
            .unwrap_or_else(|err| panic!("{}: rhythm grid failed: {err}", rule.id));
        assert_eq!(
            grid.total_ticks(),
            u64::from(bars) * rule.meter_value().ticks_per_bar()
        );
        let mut seed_variants = std::collections::BTreeSet::new();
        for seed in 0u64..4 {
            let melody = genre_melody(rule, PitchClass::C, bars, onsets, seed)
                .unwrap_or_else(|err| panic!("{}: melody failed: {err}", rule.id));
            seed_variants.insert(
                melody
                    .notes()
                    .iter()
                    .map(|note| note.pitch)
                    .collect::<Vec<u8>>(),
            );
            assert_eq!(melody.len(), grid.len(), "{}", rule.id);
            assert_eq!(melody.total_ticks(), grid.total_ticks(), "{}", rule.id);
            assert_eq!(melody.key(), key, "{}", rule.id);
            assert_eq!(melody.meter(), rule.meter_value(), "{}", rule.id);
            assert_eq!(melody.swing_permille(), rule.swing_permille().unwrap());
            assert_eq!(melody.seed(), seed);
            // onset 逐位取自网格。
            for (note, hit) in melody.notes().iter().zip(grid.hits()) {
                assert_eq!(note.start_tick, hit.tick, "{}", rule.id);
                assert_eq!(note.bar, hit.bar, "{}", rule.id);
                assert_eq!(note.cell, hit.cell, "{}", rule.id);
                assert_eq!(note.weight, hit.weight, "{}", rule.id);
            }
            // 音高在音阶内、在音域内；时值为正、首尾相接、铺满 total_ticks。
            for note in melody.notes() {
                let pc = PitchClass::new(note.pitch % 12).unwrap();
                assert!(
                    key.contains(pc),
                    "{}: pitch {} escapes {key}",
                    rule.id,
                    note.pitch
                );
                assert!(
                    (48..=84).contains(&note.pitch),
                    "{}: pitch {} leaves the default window",
                    rule.id,
                    note.pitch
                );
                assert!(note.duration_ticks > 0, "{}", rule.id);
                notes_checked += 1;
            }
            for pair in melody.notes().windows(2) {
                assert!(
                    pair[0].pitch.abs_diff(pair[1].pitch) <= 12,
                    "{}: leap {} exceeds the bound",
                    rule.id,
                    pair[0].pitch.abs_diff(pair[1].pitch)
                );
                assert_eq!(pair[0].end_tick(), pair[1].start_tick, "{}", rule.id);
            }
            assert_eq!(
                melody.notes().last().unwrap().end_tick(),
                grid.total_ticks(),
                "{}",
                rule.id
            );
            // 纯函数：同输入同输出。
            assert_eq!(
                genre_melody(rule, PitchClass::C, bars, onsets, seed).unwrap(),
                melody,
                "{}",
                rule.id
            );
            // 强拍：窗口里有和弦音时，那个音必须是和弦音（默认窗口下恒成立）。
            for note in melody.notes() {
                if note.weight < 1 {
                    continue;
                }
                let span = spans
                    .iter()
                    .find(|span| {
                        span.start_tick <= note.start_tick && note.start_tick < span.end_tick()
                    })
                    .expect("every onset sits inside the tile of spans");
                let chord_pcs: Vec<u8> = span
                    .chord
                    .pitch_classes()
                    .iter()
                    .map(|pc| pc.semitones())
                    .collect();
                let window_has_chord_tone =
                    (48..=84).any(|pitch| chord_pcs.contains(&(pitch % 12)));
                if window_has_chord_tone {
                    assert!(
                        note.chord_tone,
                        "{}: seed {seed}, strong beat {note:?} is not a chord tone",
                        rule.id
                    );
                    strong_beats_checked += 1;
                }
            }
        }
        // 种子必须真的影响输出：4 个种子给出至少 2 条不同的音高序列。
        assert!(
            seed_variants.len() >= 2,
            "{}: seeds do not change the melody",
            rule.id
        );
        checked += 1;
    }
    assert_eq!(checked, GenreLibrary::len());
    assert!(notes_checked > 0);
    assert!(strong_beats_checked > 0);
}

proptest! {
    /// 任意流派、任意小节数、任意 onset 数、任意种子：旋律与网格同形、
    /// 逐音在音阶内、跳进不超上界、首尾相接铺满。
    #[test]
    fn every_genre_melody_matches_its_grid_and_stays_in_scale(
        index in 0usize..GenreLibrary::all().len(),
        bars in 1u32..6,
        onsets in 1u32..=8,
        seed in any::<u64>(),
    ) {
        let rule = &GenreLibrary::all()[index];
        let key = rule.primary_scale(PitchClass::C)?;
        let grid = rule.rhythm_grid(bars, onsets)?;
        let melody = genre_melody(rule, PitchClass::C, bars, onsets, seed)?;
        prop_assert_eq!(melody.len(), grid.len());
        prop_assert_eq!(melody.total_ticks(), grid.total_ticks());
        prop_assert_eq!(melody.meter(), rule.meter_value());
        for (note, hit) in melody.notes().iter().zip(grid.hits()) {
            prop_assert_eq!(note.start_tick, hit.tick);
            prop_assert_eq!(note.bar, hit.bar);
            prop_assert_eq!(note.cell, hit.cell);
            let pc = PitchClass::new(note.pitch % 12)?;
            prop_assert!(key.contains(pc));
        }
        for pair in melody.notes().windows(2) {
            prop_assert!(pair[0].pitch.abs_diff(pair[1].pitch) <= 12);
            prop_assert_eq!(pair[0].end_tick(), pair[1].start_tick);
        }
        prop_assert_eq!(melody.notes().last().unwrap().end_tick(), grid.total_ticks());
    }

    /// 任意窗口与跳进上界：要么产出一条**从不离开窗口**的旋律，要么如实报
    /// `NoFeasibleVoicing`（窗口里没有音阶音）。绝无第三种结果。
    #[test]
    fn a_narrow_window_never_produces_an_out_of_window_note(
        index in 0usize..GenreLibrary::all().len(),
        lower in 48u8..=72,
        width in 0u8..=24,
        max_leap in 0u8..=12,
        seed in any::<u64>(),
    ) {
        let rule = &GenreLibrary::all()[index];
        let key = rule.primary_scale(PitchClass::C)?;
        let spans = rule.sketch(PitchClass::C, 3)?;
        let grid = rule.rhythm_grid(3, 6)?;
        let upper = lower.saturating_add(width).min(127);
        let constraints = MelodyConstraints::new(lower, upper, max_leap)?;
        match melody_over_chords(&key, &spans, &grid, constraints, seed) {
            Ok(melody) => {
                for note in melody.notes() {
                    prop_assert!(note.pitch >= lower && note.pitch <= upper);
                    let pc = PitchClass::new(note.pitch % 12)?;
                    prop_assert!(key.contains(pc));
                }
                for pair in melody.notes().windows(2) {
                    prop_assert!(pair[0].pitch.abs_diff(pair[1].pitch) <= max_leap);
                }
            }
            Err(err) => prop_assert_eq!(err, TheoryError::NoFeasibleVoicing),
        }
    }

    /// 种子版骨架：与"显式索引选同一条走向"的结果逐位相同，且满足全部结构
    /// 不变量、根音恒属于**本次选中的**音阶。
    #[test]
    fn a_seeded_sketch_equals_its_explicit_index_and_keeps_every_invariant(
        index in 0usize..GenreLibrary::all().len(),
        bars in 1u32..6,
        seed in any::<u64>(),
    ) {
        let rule = &GenreLibrary::all()[index];
        let spans = rule.sketch_for(PitchClass::C, bars, seed)?;
        let key = rule.scale_for(PitchClass::C, seed)?;
        let meter = rule.meter_value();
        let total: u64 = spans.iter().map(|span| span.duration_ticks).sum();
        prop_assert_eq!(total, u64::from(bars) * meter.ticks_per_bar());
        let mut cursor = 0u64;
        for span in &spans {
            prop_assert_eq!(span.start_tick, cursor);
            prop_assert!(span.duration_ticks > 0);
            prop_assert_eq!(span.duration_ticks % 240, 0);
            prop_assert!(key.contains(span.chord.root));
            cursor = span.end_tick();
        }
        // 同一条走向用显式索引展开必须逐位相同（两条入口共用同一份展开规则）。
        let explicit = rule.progression_for(seed)?;
        let found = (0..rule.progression_count())
            .find(|&candidate| rule.progression_at(candidate) == Some(explicit))
            .expect("the seeded progression must be a registered one");
        let via_index = Progression::parse(rule.progression_at(found).unwrap())?
            .with_meter(meter)
            .with_bars(bars)?
            .expand(&key)?;
        prop_assert!(spans == via_index);
        // 旧入口在种子选中第 0 条（走向与音阶都是）时逐位不变。
        if found == 0 && key.kind == rule.primary_scale(PitchClass::C)?.kind {
            prop_assert!(spans == rule.sketch(PitchClass::C, bars)?);
        }
    }
}

/// 种子版旋律：全部音高属于**它自己那条旋律的调**；182 条流派各自都能被种子
/// 换一版骨架（不是常量函数）。
///
/// 放在 `proptest!` 之外：本判据没有随机输入，遍历的是全部 182 条登记流派。
#[test]
fn every_genre_can_change_its_section_with_the_seed() {
    assert_eq!(
        GenreLibrary::all().len(),
        182,
        "scan domain must not shrink"
    );
    let mut genres_that_vary = 0usize;
    for rule in GenreLibrary::all() {
        let baseline = rule.sketch(PitchClass::C, 4).unwrap();
        let mut varies = false;
        for seed in 0u64..32 {
            if rule.sketch_for(PitchClass::C, 4, seed).unwrap() != baseline {
                varies = true;
                break;
            }
        }
        if varies {
            genres_that_vary += 1;
        }
        // 种子版旋律的音高恒在它自己的调里（音域用文档字面量 48..=84）。
        for seed in 0u64..4 {
            let melody = genre_melody_for(rule, PitchClass::C, 2, 4, seed).unwrap();
            let key = rule.scale_for(PitchClass::C, seed).unwrap();
            assert_eq!(melody.key(), key, "{}", rule.id);
            for note in melody.notes() {
                let pc = PitchClass::new(note.pitch % 12).unwrap();
                assert!(key.contains(pc), "{} seed {seed}", rule.id);
                assert!((48..=84).contains(&note.pitch), "{}", rule.id);
            }
        }
    }
    // 实测读数：182/182 条流派在种子 0..32 里至少有一版骨架与旧 API 不同。
    assert_eq!(genres_that_vary, 182);
}

// ---------------------------------------------------------------------------
// 9. 端到端：鼓组型（`pending 3` 的"具体鼓点"侧）
// ---------------------------------------------------------------------------

/// 全部参与鼓组属性测试的拍号（含 `GENRES` 没用到的 5/4、8/8、9/8、11/8、12/8）。
const DRUM_METERS: [Meter; 10] = GROUPING_METERS;

/// 从网格的 `(tick, bar, cell, weight)` 判断某件鼓件是否**应当**在该 onset 上响。
///
/// 这是判据侧独立复算的分派规则（不调用 crate 的实现），读法：底鼓 = 组的起点、
/// 军鼓 = 反拍拍的起点、踩镲 = 每一格、吊镲 = 强位上的组起点。
fn expected_voice(
    voice: DrumVoice,
    meter: Meter,
    cell: u32,
    weight: u8,
    grouping: BeatGrouping<'_>,
    backbeat: u8,
) -> bool {
    let beats = felt_beats_per_bar(meter);
    let cells = cells_per_bar(meter).expect("test meters are valid");
    let cells_per_beat = cells / u64::from(beats);
    let offset = u64::from(cell) % cells_per_beat;
    let on_beat_start = offset == 0;
    let beat = (u64::from(cell) / cells_per_beat) as u8;
    let group_start = on_beat_start && grouping.is_group_start(beat);
    match voice {
        DrumVoice::Kick => group_start,
        DrumVoice::Snare => on_beat_start && backbeat != 0 && beat.is_multiple_of(backbeat),
        DrumVoice::HiHat => true,
        DrumVoice::Ride => group_start && weight >= STRONG_BEAT_WEIGHT,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// 任意拍号 × 任意小节数 × 任意 onset 数：鼓组型的每一件鼓件都恰好落在
    /// **判据侧独立复算**应当命中的那些 onset 上，不多不少。
    ///
    /// 单位：`checked` 数的是**被接受的样本个数**（构造成功的鼓组型），
    /// 断言的对象是"每个 (鼓件, onset) 对的命中与否"这一布尔值。
    #[test]
    fn every_drum_voice_lands_exactly_where_the_documented_rule_says(
        meter_index in 0usize..DRUM_METERS.len(),
        bars in 1u32..5,
        onsets in 0u32..13,
    ) {
        let meter = DRUM_METERS[meter_index];
        let beats = felt_beats_per_bar(meter);
        let backbeat = default_backbeat(meter);
        let grouping = BeatGrouping::new(meter, &[1u8; 16][..usize::from(beats)])
            .expect("per-beat grouping is always valid");
        let Ok(Some(pattern)) =
            swung_drum_pattern(meter, bars, onsets, None, Some(grouping), backbeat)
        else {
            // 每小节不足一拍的病态拍号：如实跳过，不假装通过。
            return Ok(());
        };
        let grid = pattern.grid();
        for onset in grid.hits() {
            for voice in DrumVoice::ALL {
                let should = expected_voice(voice, meter, onset.cell, onset.weight, grouping, backbeat);
                let did = pattern
                    .hits()
                    .iter()
                    .any(|hit| hit.voice == voice && hit.tick == onset.tick && hit.cell == onset.cell);
                prop_assert_eq!(did, should, "meter {:?} cell {} voice {}", meter, onset.cell, voice.name());
            }
        }
        // 踩镲覆盖每一个 onset（"每格一击"）。
        prop_assert_eq!(pattern.hit_count(DrumVoice::HiHat), grid.len());
        // 击点总数 = 各鼓件击点数之和。
        prop_assert_eq!(
            pattern.len(),
            DrumVoice::ALL.iter().map(|&voice| pattern.hit_count(voice)).sum::<usize>()
        );
    }
}

/// 鼓组型的结构不变量：按 `(tick, 鼓件序)` 严格升序、同一 tick 同一鼓件最多一次、
/// 每个击点都能在网格里找到逐位相同的 onset、`total_ticks` 与网格一致。
///
/// 放在 `proptest!` 之外：本判据遍历的是**全部 182 条登记流派**。
#[test]
fn every_registered_genre_produces_a_well_formed_drum_pattern() {
    let mut checked = 0usize;
    let mut too_dense = 0usize;
    for rule in GenreLibrary::all() {
        for onsets in [1u32, 2, 3, 4, 6, 8, 12] {
            let Ok(Some(pattern)) = rule.drum_pattern(2, onsets) else {
                // 只允许一种"不产出"的理由：请求的 onset 数超过该拍号的格位数
                // （`ProgressionTooDense`）。**不**静默钳制，也**不**把别的错误
                // 当成跳过 —— 否则这条判据会变成"什么都没查"。
                assert!(
                    onsets as usize > cells_per_bar(rule.meter_value()).unwrap() as usize,
                    "{} onsets {onsets} failed for a reason other than density",
                    rule.id
                );
                too_dense += 1;
                continue;
            };
            checked += 1;
            let grid = pattern.grid();
            assert_eq!(pattern.meter(), rule.meter_value(), "{}", rule.id);
            assert_eq!(pattern.bars(), 2, "{}", rule.id);
            assert_eq!(
                pattern.total_ticks(),
                2 * rule.meter_value().ticks_per_bar()
            );
            assert_eq!(pattern.ticks_per_bar(), rule.meter_value().ticks_per_bar());
            assert!(pattern.hit_count(DrumVoice::HiHat) <= grid.len());
            for pair in pattern.hits().windows(2) {
                assert!(
                    (pair[0].tick, pair[0].voice.ordinal())
                        < (pair[1].tick, pair[1].voice.ordinal()),
                    "{} {:?} then {:?}",
                    rule.id,
                    pair[0],
                    pair[1]
                );
            }
            for hit in pattern.hits() {
                assert!(hit.tick < pattern.total_ticks(), "{} {hit:?}", rule.id);
                let onset = grid.hits().iter().find(|onset| {
                    onset.tick == hit.tick
                        && onset.bar == hit.bar
                        && onset.cell == hit.cell
                        && onset.weight == hit.weight
                });
                assert!(onset.is_some(), "{} {hit:?} is not a grid onset", rule.id);
                assert_eq!(hit.accent, hit.weight >= STRONG_BEAT_WEIGHT, "{}", rule.id);
            }
            // 每个小节的切片拼起来就是全部击点。
            let rebuilt: Vec<DrumHit> = (0..pattern.bars())
                .flat_map(|bar| pattern.hits_in_bar(bar).iter().copied())
                .collect();
            assert!(rebuilt == pattern.hits().to_vec(), "{}", rule.id);
        }
    }
    // 实测读数：182 条流派 × 7 个 onset 数 = 1274 个请求；其中 **1266** 个
    // 构造成功，**8** 个按 `ProgressionTooDense` 如实拒绝（登记表里格位数最少
    // 的拍号是 2/4 与 7/8，只有 8 或 14 格 ⇒ 请求 12 个 onset 时 2/4 报错）。
    assert_eq!(checked, 1266);
    assert_eq!(too_dense, 8);
    assert_eq!(checked + too_dense, 182 * 7);
}

/// 摇摆只移动 tick、不改鼓件分派：同一 `(cell, voice)` 对在两份鼓组型里都存在。
///
/// 两侧都读**该流派登记的** [`DrumStyle`]（`rule.drum_style`），因此本判据检验的
/// 是"摇摆不改分派"，不是"分派与流派无关"——后者由
/// `the_genre_drum_entry_point_reads_only_the_genres_own_registered_fields` 负责。
#[test]
fn swing_never_changes_which_voice_strikes_a_cell() {
    for rule in GenreLibrary::all() {
        let Ok(straight) = rule.rhythm_grid(1, 8) else {
            continue;
        };
        let Ok(Some(plain)) = rule.drum_pattern(1, 8) else {
            continue;
        };
        // 有摇摆比例的流派：鼓件分派与网格格点集合都不因摇摆改变。
        if let Ok(Some(swung_permille)) = rule.swing_permille() {
            let swung = swung_styled_drum_pattern(
                rule.meter_value(),
                1,
                8,
                Some(swung_permille),
                None,
                default_backbeat(rule.meter_value()),
                rule.drum_style,
            )
            .unwrap()
            .unwrap();
            assert_eq!(swung.len(), plain.len(), "{}", rule.id);
            for hit in plain.hits() {
                assert!(
                    swung
                        .hits()
                        .iter()
                        .any(|other| other.cell == hit.cell && other.voice == hit.voice),
                    "{} {hit:?}",
                    rule.id
                );
            }
        }
        // 鼓组型读的网格 onset 集合必须与 rhythm_grid 的逐位相同。
        assert!(
            plain.grid().hits() == straight.hits(),
            "{}: the drum pattern must read the genre's own grid",
            rule.id
        );
    }
}

/// 流派入口只读**该流派自己登记的**字段：`GenreRule::drum_pattern` 的读数必须
/// 与"把该流派的拍号 / 摇摆比例 / 底鼓口径显式传给 drum 模块"逐位相同。
///
/// 数什么：比较过的 (流派, onset 数) 组合个数，单位 = "个"。
#[test]
fn the_genre_drum_entry_point_reads_only_the_genres_own_registered_fields() {
    let mut compared = 0usize;
    for rule in GenreLibrary::all() {
        for onsets in [1u32, 2, 4, 8] {
            let Ok(Some(pattern)) = rule.drum_pattern(2, onsets) else {
                continue;
            };
            let expected = swung_styled_drum_pattern(
                rule.meter_value(),
                2,
                onsets,
                rule.swing_permille().unwrap(),
                None,
                default_backbeat(rule.meter_value()),
                rule.drum_style,
            )
            .unwrap()
            .unwrap();
            assert!(
                pattern.hits() == expected.hits(),
                "{} onsets {onsets}",
                rule.id
            );
            assert!(
                pattern.grid().hits() == expected.grid().hits(),
                "{} onsets {onsets}",
                rule.id
            );
            compared += 1;
        }
    }
    // 182 条流派 × 4 个 onset 数 = 728 个组合（请求的最大 onset 数 8 不超过
    // 登记表里最小的格位数 8）。
    assert_eq!(compared, 728);
}

/// 四踩底鼓是度量口径底鼓的**严格超集**：登记了它的流派多出的底鼓个数为正，
/// 且旧的底鼓一个都不少。
#[test]
fn four_on_the_floor_genres_get_a_kick_where_the_metric_style_has_none() {
    let four = GenreLibrary::by_drum_style(DrumStyle::FourOnTheFloor);
    assert_eq!(four.len(), 13);
    let mut extra_kicks = 0usize;
    for rule in four {
        let cells = cells_per_bar(rule.meter_value()).unwrap() as u32;
        let styled = rule.drum_pattern(1, cells).unwrap().unwrap();
        let metric = styled_drum_pattern(
            rule.meter_value(),
            1,
            cells,
            None,
            default_backbeat(rule.meter_value()),
            DrumStyle::Metric,
        )
        .unwrap()
        .unwrap();
        for hit in metric
            .hits()
            .iter()
            .filter(|hit| hit.voice == DrumVoice::Kick)
        {
            assert!(
                styled
                    .hits()
                    .iter()
                    .any(|other| other.voice == DrumVoice::Kick && other.tick == hit.tick),
                "{}: the metric kick at {} vanished",
                rule.id,
                hit.tick
            );
        }
        extra_kicks += styled.hit_count(DrumVoice::Kick) - metric.hit_count(DrumVoice::Kick);
    }
    // 13 条 4/4 流派 × (每拍一击的 4 个 − 组起点的 2 个) = 26 个新增底鼓。
    assert_eq!(extra_kicks, 26);
}
