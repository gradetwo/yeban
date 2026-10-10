//! 跨模块**黄金表**：把 9 条手写 `Display`、每个公开枚举的变体、每个公开常量的
//! 取值、以及整张流派登记表，各钉成一张可逐行核对（且**位置敏感**）的表。
//!
//! ## 为什么需要这一层（形态 D 第五/六批的实测）
//!
//! * 第四批补了**求和**聚合读数，第五批发现它抓不住"一条 +1、另一条 -1"的
//!   抵消式改动，于是补了**多重集**直方图；第六批又构造出**纯互换**两条登记值
//!   —— 互换不改求和、也不改多重集 ⇒ 两层都躲得过。本文件的逐条表是第三层，
//!   它**按位置**钉住整张登记表，互换因此无处可躲。
//! * 9 条手写 `Display` 里有 5 条曾被"改坏而没有判据变红"（第四批实测）；
//!   `impl Eq for GenreRule {}` 整条删掉同样全绿（第五批实测，本文件用
//!   **trait 约束**把它变成编译期契约）。
//! * 每个公开常量的**取值**此前没有判据：`SEARCH_BEAM = 24 → 25` 四道闸门全绿
//!   （第六批实测），而其余常量是被间接判据守住的。
//!
//! ⚠ 注入纪律（R44①）：本文件的黄金表会让被钉文本**出现两次**，因此针对这些
//! 契约的注入必须**限定上下文**（按行锚定生产代码，不按文本搜全局）。

use yeban_theory::chord::{Chord, ChordKind, Tonality};
use yeban_theory::drum::{DRUM_VOICE_COUNT, DrumStyle, DrumVoice};
use yeban_theory::genre::{GenreLibrary, GenreRule};
use yeban_theory::melody::{
    CHORD_TONE_WEIGHT_FLOOR, MELODY_LOWER_BOUND, MELODY_MAX_LEAP, MELODY_UPPER_BOUND,
};
use yeban_theory::pitch::{Interval, NoteName, Pitch, PitchClass, SpelledPitch};
use yeban_theory::progression::{Degree, MIN_DURATION_TICKS, PPQ, Progression, RomanQuality};
use yeban_theory::rhythm::{
    BEAT_WEIGHT, GRID_CELL_TICKS, MAX_METRIC_WEIGHT, OFFBEAT_WEIGHT, STRONG_BEAT_WEIGHT,
    SWING_PAIR_TICKS,
};
use yeban_theory::scale::{Scale, ScaleKind};
use yeban_theory::swing::{SWING_PERMILLE_MAX, SWING_PERMILLE_STRAIGHT};

/// 9 条手写 `Display`，每条一行。加一条 `Display` 而不加一行 ⇒ 长度断言变红。
#[test]
fn display_golden_table_covers_every_manual_display_impl() {
    let rendered: Vec<String> = vec![
        // 1. impl fmt::Display for Chord
        Chord::new(PitchClass::C, ChordKind::Major).to_string(),
        // 2. impl fmt::Display for NoteName
        NoteName::new(6, 1).unwrap().to_string(),
        // 3. impl fmt::Display for SpelledPitch
        "C#4".parse::<SpelledPitch>().unwrap().to_string(),
        // 4. impl fmt::Display for PitchClass
        PitchClass::FS.to_string(),
        // 5. impl fmt::Display for Pitch
        Pitch::C4.to_string(),
        // 6. impl fmt::Display for Interval
        Interval::PERFECT_FIFTH.to_string(),
        // 7. impl core::fmt::Display for Degree
        Degree::parse("bVII").unwrap().to_string(),
        // 8. impl core::fmt::Display for Progression
        Progression::parse("I-V-vi-IV").unwrap().to_string(),
        // 9. impl core::fmt::Display for Scale
        Scale::new(PitchClass::C, ScaleKind::Major).to_string(),
    ];
    assert_eq!(
        rendered.len(),
        9,
        "the crate has exactly 9 manual `Display` impls"
    );
    assert_eq!(
        rendered,
        vec![
            "C",
            "B#",
            "C#4",
            "F#",
            "C4",
            "P5",
            "bVII",
            "I-V-vi-IV",
            "C major"
        ]
    );
    // 与具名读数交叉核对（`Display` 都是委派，两者必须逐字相同）。
    assert_eq!(
        rendered[0],
        Chord::new(PitchClass::C, ChordKind::Major).symbol()
    );
    assert_eq!(rendered[5], Interval::PERFECT_FIFTH.name());
    assert_eq!(rendered[6], Degree::parse("bVII").unwrap().symbol());
    assert_eq!(
        rendered[8],
        format!("{} {}", PitchClass::C, ScaleKind::Major.name())
    );
}

/// 每个公开枚举的变体都用**没有通配臂**的 `match` 逐个列出 ⇒ 上游新增一个变体
/// 会让本文件**编译失败**，强迫作者把新变体补进黄金表。
#[test]
fn enum_variants_are_exhaustively_matched() {
    fn chord_kind_index(kind: ChordKind) -> usize {
        match kind {
            ChordKind::Major => 0,
            ChordKind::Minor => 1,
            ChordKind::Diminished => 2,
            ChordKind::Augmented => 3,
            ChordKind::Sus2 => 4,
            ChordKind::Sus4 => 5,
            ChordKind::Six => 6,
            ChordKind::Dominant7 => 7,
            ChordKind::Major7 => 8,
            ChordKind::Minor7 => 9,
            ChordKind::HalfDiminished7 => 10,
            ChordKind::Diminished7 => 11,
            ChordKind::MinorMajor7 => 12,
            ChordKind::Dominant9 => 13,
            ChordKind::Major9 => 14,
            ChordKind::Minor9 => 15,
            ChordKind::Dominant11 => 16,
            ChordKind::Dominant13 => 17,
            ChordKind::Add9 => 18,
            ChordKind::SixNine => 19,
            ChordKind::Dominant7Sus4 => 20,
        }
    }
    fn tonality_index(tonality: Tonality) -> usize {
        match tonality {
            Tonality::SharpMajor => 0,
            Tonality::FlatMajor => 1,
            Tonality::Minor => 2,
        }
    }
    fn scale_kind_index(kind: ScaleKind) -> usize {
        match kind {
            ScaleKind::Major => 0,
            ScaleKind::Ionian => 1,
            ScaleKind::NaturalMinor => 2,
            ScaleKind::Aeolian => 3,
            ScaleKind::HarmonicMinor => 4,
            ScaleKind::MelodicMinor => 5,
            ScaleKind::Dorian => 6,
            ScaleKind::Phrygian => 7,
            ScaleKind::Lydian => 8,
            ScaleKind::Mixolydian => 9,
            ScaleKind::Locrian => 10,
            ScaleKind::PentatonicMajor => 11,
            ScaleKind::PentatonicMinor => 12,
            ScaleKind::Blues => 13,
            ScaleKind::WholeTone => 14,
            ScaleKind::Chromatic => 15,
        }
    }
    fn roman_quality_index(quality: RomanQuality) -> usize {
        match quality {
            RomanQuality::Major => 0,
            RomanQuality::Minor => 1,
            RomanQuality::Diminished => 2,
            RomanQuality::Augmented => 3,
            RomanQuality::HalfDiminished => 4,
        }
    }
    // 每个变体都必须被上面的 `match` 接受（这些调用的存在本身就证明表是全的）。
    assert_eq!(chord_kind_index(ChordKind::Major), 0);
    assert_eq!(chord_kind_index(ChordKind::Dominant7Sus4), 20);
    assert_eq!(tonality_index(Tonality::Minor), 2);
    assert_eq!(scale_kind_index(ScaleKind::Major), 0);
    assert_eq!(scale_kind_index(ScaleKind::Chromatic), 15);
    assert_eq!(roman_quality_index(RomanQuality::HalfDiminished), 4);
    // 变体总数（`Display`/`name` 的臂数必须与这些数字一致）。
    assert_eq!(DrumVoice::ALL.len(), DRUM_VOICE_COUNT);
    assert_eq!(DrumStyle::ALL.len(), 2);
    assert_eq!(ChordKind::from_suffix("").unwrap(), ChordKind::Major);
}

/// 公开常量的**取值**逐条钉住（不是常量之间互比）。
///
/// 口径：把每个常量的读数放进**数组**再与字面量数组比对 —— 数组的元素来自
/// 常量，因此"常量本身取值"被钉住，而比较对象是字面量（不是另一个常量）。
/// 派生关系（`GRID_CELL_TICKS == MIN_DURATION_TICKS` 之类）在数组比对之外单独钉。
#[test]
fn crate_constants_are_pinned_to_their_documented_values() {
    let ticks: [u64; 4] = [PPQ, MIN_DURATION_TICKS, GRID_CELL_TICKS, SWING_PAIR_TICKS];
    assert_eq!(ticks, [960u64, 240, 240, 480]);
    let weights: [u8; 4] = [
        MAX_METRIC_WEIGHT,
        STRONG_BEAT_WEIGHT,
        BEAT_WEIGHT,
        OFFBEAT_WEIGHT,
    ];
    assert_eq!(weights, [8u8, 3, 2, 1]);
    let swing: [u16; 2] = [SWING_PERMILLE_STRAIGHT, SWING_PERMILLE_MAX];
    assert_eq!(swing, [500u16, 1000]);
    let melody: [u8; 4] = [
        MELODY_LOWER_BOUND,
        MELODY_UPPER_BOUND,
        MELODY_MAX_LEAP,
        CHORD_TONE_WEIGHT_FLOOR,
    ];
    assert_eq!(melody, [48u8, 84, 12, 1]);
    let counts: [usize; 2] = [yeban_theory::voice_leading::SEARCH_BEAM, DRUM_VOICE_COUNT];
    assert_eq!(counts, [24usize, 4]);
    // 派生关系（跨常量，仍然不是自比）：把左侧读进局部变量再比。
    let grid_cell = GRID_CELL_TICKS;
    let min_duration = MIN_DURATION_TICKS;
    let swing_pair = SWING_PAIR_TICKS;
    assert_eq!(grid_cell, min_duration);
    assert_eq!(min_duration, PPQ / 4);
    assert_eq!(swing_pair, PPQ / 2);
    assert!(weights[0] > weights[1]);
    assert!(weights[1] > weights[2]);
    assert!(weights[2] > weights[3]);
    assert!(swing[1] > swing[0]);
    assert!(melody[0] < melody[1]);
}

/// `GenreRule` 的公开 trait 界面（第五批实测：`impl Eq` 整条删掉时四道闸门全绿，
/// 且 `yeban-mcp` 只用 `&GenreRule` ＋ 方法，因此**仓内不可观测**）。
/// 把它变成**编译期**契约：删掉 `impl Eq` 会让本判据**编译失败**。
#[test]
fn genre_rule_keeps_its_public_trait_bounds() {
    fn assert_copy<T: Copy>() {}
    fn assert_debug<T: core::fmt::Debug>() {}
    fn assert_eq_bound<T: Eq>() {}
    fn assert_send_sync<T: Send + Sync + 'static>() {}
    assert_copy::<GenreRule>();
    assert_debug::<GenreRule>();
    assert_eq_bound::<GenreRule>();
    assert_send_sync::<GenreRule>();
    // 运行期读数：相等是逐字段的（与第三批判据互补）。
    let rule = GenreLibrary::all()[0];
    assert_eq!(rule, rule);
}

/// 整张流派登记表的**逐条、位置敏感**黄金表。
///
/// 前两层聚合读数（第四批求和、第五批多重集）都躲得过**纯互换**两条登记值
/// （第六批实测 S01/S02/S03 全绿），本表按**位置**钉住 `(id, BPM, 拍号, 密度,
/// 摇摆千分比)` ⇒ 互换、单改、抵消式改动都会变红。
/// 同时它也钉住了一条单改 `swing` 的注入（S08，此前全绿）。
/// 登记表黄金表的一行：`(id, BPM 区间, 拍号, 每小节音符数区间, 摇摆千分比)`。
type RegistryRow = (&'static str, (u16, u16), (u8, u8), (u8, u8), Option<u16>);

#[test]
fn registry_per_entry_golden_table_is_position_sensitive() {
    const TABLE: [RegistryRow; 182] = [
        ("gregorian_chant", (50, 76), (4, 4), (4, 12), None),
        ("renaissance_polyphony", (60, 88), (4, 4), (16, 40), None),
        ("baroque_chorale", (56, 84), (4, 4), (12, 32), None),
        ("baroque_fugue", (66, 104), (4, 4), (24, 64), None),
        ("baroque_suite", (72, 132), (3, 4), (16, 48), None),
        ("classical_sonata", (80, 132), (4, 4), (16, 48), None),
        ("classical_minuet", (108, 132), (3, 4), (12, 32), None),
        ("waltz", (84, 180), (3, 4), (12, 36), None),
        ("march", (100, 140), (2, 4), (8, 24), None),
        ("romantic_lied", (56, 96), (4, 4), (12, 36), None),
        ("nocturne", (52, 84), (4, 4), (16, 48), None),
        ("etude", (80, 176), (4, 4), (32, 96), None),
        ("impressionism", (54, 92), (4, 4), (16, 56), None),
        ("impressionist_piano", (50, 88), (4, 4), (20, 64), None),
        ("orchestral_film_score", (60, 140), (4, 4), (16, 64), None),
        ("epic_trailer", (70, 130), (4, 4), (16, 48), None),
        ("minimalism", (72, 160), (4, 4), (32, 128), None),
        ("hymn", (60, 92), (4, 4), (8, 24), None),
        ("anthem", (64, 108), (4, 4), (8, 28), None),
        ("carol", (76, 132), (3, 4), (8, 24), None),
        ("lullaby", (52, 80), (3, 4), (4, 16), None),
        ("opera_aria", (60, 104), (4, 4), (12, 48), None),
        ("operetta", (88, 152), (3, 4), (12, 36), Some(560)),
        ("ballet", (64, 144), (4, 4), (16, 56), None),
        ("passacaglia", (56, 88), (3, 4), (16, 48), None),
        ("chaconne", (60, 96), (3, 4), (16, 56), None),
        ("toccata", (92, 152), (4, 4), (32, 128), None),
        ("prelude", (66, 120), (4, 4), (24, 96), None),
        ("sonatina", (88, 132), (4, 4), (16, 40), None),
        ("jazz_swing", (120, 260), (4, 4), (8, 32), Some(660)),
        ("big_band", (110, 220), (4, 4), (16, 48), Some(660)),
        ("bebop", (180, 320), (4, 4), (32, 128), Some(580)),
        ("hard_bop", (140, 260), (4, 4), (24, 96), Some(640)),
        ("cool_jazz", (88, 180), (4, 4), (12, 48), Some(600)),
        ("modal_jazz", (96, 200), (4, 4), (8, 32), Some(600)),
        ("free_jazz", (80, 240), (4, 4), (32, 160), None),
        ("bossa_nova", (96, 140), (4, 4), (8, 32), Some(540)),
        ("latin_jazz", (120, 220), (4, 4), (16, 64), None),
        ("smooth_jazz", (80, 116), (4, 4), (8, 24), Some(540)),
        ("jazz_waltz", (120, 220), (3, 4), (12, 48), Some(620)),
        ("gypsy_jazz", (140, 280), (4, 4), (24, 96), Some(660)),
        ("ragtime", (88, 140), (2, 4), (24, 80), None),
        ("stride_piano", (110, 200), (4, 4), (32, 96), Some(620)),
        ("boogie_woogie", (100, 180), (4, 4), (32, 96), Some(620)),
        ("blues", (60, 160), (4, 4), (8, 32), Some(640)),
        ("delta_blues", (58, 104), (4, 4), (4, 16), Some(580)),
        ("chicago_blues", (80, 160), (4, 4), (12, 48), Some(620)),
        ("jump_blues", (130, 220), (4, 4), (16, 64), Some(660)),
        ("blues_rock", (90, 170), (4, 4), (12, 40), Some(560)),
        ("gospel", (68, 140), (4, 4), (12, 48), Some(580)),
        ("spiritual", (60, 104), (4, 4), (6, 20), None),
        ("work_song", (72, 132), (4, 4), (4, 16), None),
        ("field_holler", (52, 92), (4, 4), (2, 10), None),
        ("soul", (72, 140), (4, 4), (12, 48), Some(560)),
        ("motown", (100, 148), (4, 4), (16, 48), None),
        ("funk", (90, 130), (4, 4), (24, 96), None),
        ("p_funk", (96, 128), (4, 4), (24, 80), None),
        ("disco", (112, 132), (4, 4), (24, 64), None),
        ("boogie", (108, 136), (4, 4), (24, 64), None),
        ("contemporary_rnb", (64, 104), (4, 4), (12, 48), Some(540)),
        ("neo_soul", (68, 104), (4, 4), (16, 64), Some(560)),
        ("quiet_storm", (60, 92), (4, 4), (8, 32), None),
        ("doo_wop", (76, 128), (4, 4), (8, 24), None),
        ("rock_and_roll", (120, 200), (4, 4), (12, 40), Some(560)),
        ("rockabilly", (140, 220), (4, 4), (16, 56), Some(580)),
        ("surf_rock", (120, 176), (4, 4), (16, 48), None),
        ("garage_rock", (120, 180), (4, 4), (16, 48), None),
        ("psychedelic_rock", (76, 140), (4, 4), (16, 64), None),
        ("progressive_rock", (70, 176), (7, 8), (24, 96), None),
        ("hard_rock", (100, 160), (4, 4), (16, 48), None),
        ("heavy_metal", (100, 180), (4, 4), (24, 96), None),
        ("thrash_metal", (160, 260), (4, 4), (32, 128), None),
        ("death_metal", (140, 260), (4, 4), (48, 160), None),
        ("black_metal", (120, 240), (4, 4), (32, 128), None),
        ("doom_metal", (50, 90), (4, 4), (8, 32), None),
        ("power_metal", (130, 200), (4, 4), (32, 128), None),
        ("progressive_metal", (90, 200), (7, 8), (32, 160), None),
        ("metalcore", (130, 220), (4, 4), (32, 128), None),
        ("nu_metal", (80, 140), (4, 4), (16, 64), None),
        ("punk_rock", (140, 220), (4, 4), (16, 64), None),
        ("pop_punk", (140, 200), (4, 4), (16, 64), None),
        ("post_punk", (110, 170), (4, 4), (16, 48), None),
        ("new_wave", (110, 160), (4, 4), (16, 56), None),
        ("shoegaze", (76, 130), (4, 4), (16, 64), None),
        ("grunge", (80, 150), (4, 4), (12, 48), None),
        ("alternative_rock", (90, 150), (4, 4), (16, 56), None),
        ("indie_rock", (96, 150), (4, 4), (16, 56), None),
        ("math_rock", (110, 190), (7, 8), (32, 128), None),
        ("post_rock", (60, 130), (4, 4), (16, 80), None),
        ("emo", (110, 180), (4, 4), (16, 64), None),
        ("house", (118, 130), (4, 4), (16, 48), None),
        ("deep_house", (110, 125), (4, 4), (12, 40), None),
        ("tech_house", (122, 130), (4, 4), (24, 64), None),
        ("progressive_house", (124, 132), (4, 4), (16, 56), None),
        ("garage_house", (120, 132), (4, 4), (16, 48), Some(560)),
        ("techno", (125, 150), (4, 4), (24, 96), None),
        ("minimal_techno", (120, 132), (4, 4), (8, 32), None),
        ("trance", (128, 145), (4, 4), (32, 128), None),
        ("psytrance", (138, 150), (4, 4), (48, 160), None),
        ("hardstyle", (145, 160), (4, 4), (32, 96), None),
        ("dubstep", (138, 145), (4, 4), (16, 64), None),
        ("drum_and_bass", (165, 180), (4, 4), (32, 128), None),
        ("jungle", (155, 175), (4, 4), (32, 160), None),
        ("breakbeat", (120, 145), (4, 4), (24, 96), None),
        ("big_beat", (120, 140), (4, 4), (32, 128), None),
        ("trip_hop", (75, 100), (4, 4), (8, 32), None),
        ("downtempo", (70, 110), (4, 4), (8, 32), None),
        ("ambient", (50, 90), (4, 4), (2, 16), None),
        ("drone", (40, 76), (4, 4), (1, 8), None),
        ("new_age", (56, 92), (4, 4), (8, 32), None),
        ("synthwave", (80, 118), (4, 4), (16, 64), None),
        ("vaporwave", (60, 90), (4, 4), (8, 32), None),
        ("lo_fi_hip_hop", (70, 95), (4, 4), (8, 32), Some(560)),
        ("boom_bap", (85, 100), (4, 4), (8, 32), Some(580)),
        ("trap", (130, 150), (4, 4), (16, 96), None),
        ("drill", (138, 150), (4, 4), (16, 96), None),
        ("grime", (138, 142), (4, 4), (24, 96), None),
        ("chiptune", (110, 180), (4, 4), (32, 128), None),
        ("video_game_score", (90, 176), (4, 4), (24, 96), None),
        ("pop", (90, 130), (4, 4), (12, 48), None),
        ("dance_pop", (110, 128), (4, 4), (16, 64), None),
        ("synth_pop", (100, 140), (4, 4), (16, 64), None),
        ("ballad", (60, 88), (4, 4), (6, 24), None),
        ("power_ballad", (64, 92), (4, 4), (8, 32), None),
        ("folk", (80, 140), (4, 4), (6, 24), None),
        ("americana", (76, 132), (4, 4), (8, 32), None),
        ("country", (80, 140), (4, 4), (8, 32), Some(560)),
        ("bluegrass", (110, 180), (4, 4), (32, 96), None),
        ("honky_tonk", (100, 150), (4, 4), (16, 48), Some(580)),
        ("outlaw_country", (84, 136), (4, 4), (8, 32), Some(560)),
        ("celtic", (90, 160), (6, 8), (24, 96), None),
        ("irish_trad", (100, 180), (6, 8), (24, 96), None),
        ("jig", (110, 160), (6, 8), (24, 72), None),
        ("reel", (130, 200), (4, 4), (32, 96), None),
        ("scottish_trad", (90, 160), (4, 4), (24, 72), None),
        ("klezmer", (90, 160), (4, 4), (16, 64), Some(560)),
        ("balkan", (110, 180), (7, 8), (24, 96), None),
        ("polka", (110, 150), (2, 4), (16, 48), None),
        ("chanson", (76, 140), (3, 4), (8, 32), Some(560)),
        ("fado", (60, 100), (4, 4), (8, 32), None),
        ("cabaret", (88, 150), (4, 4), (12, 48), Some(580)),
        ("music_hall", (100, 160), (4, 4), (12, 40), None),
        ("samba", (90, 130), (2, 4), (32, 96), None),
        ("bossa_nova_brazil", (100, 136), (4, 4), (16, 56), Some(540)),
        ("choro", (110, 160), (2, 4), (32, 128), None),
        ("tango", (60, 120), (4, 4), (12, 48), None),
        ("milonga", (90, 130), (2, 4), (16, 48), None),
        ("bolero", (60, 96), (4, 4), (8, 32), None),
        ("son_cubano", (90, 140), (4, 4), (16, 64), None),
        ("salsa", (150, 220), (4, 4), (24, 96), None),
        ("merengue", (120, 180), (2, 4), (24, 80), None),
        ("bachata", (110, 150), (4, 4), (16, 64), None),
        ("cumbia", (85, 120), (4, 4), (16, 64), None),
        ("reggaeton", (88, 100), (4, 4), (16, 64), None),
        ("mariachi", (90, 150), (3, 4), (12, 48), None),
        ("ranchera", (70, 130), (3, 4), (8, 32), None),
        ("norteno", (100, 150), (2, 4), (16, 56), None),
        ("tejano", (100, 150), (4, 4), (16, 56), None),
        ("flamenco", (90, 220), (4, 4), (24, 128), None),
        ("sevillanas", (120, 180), (3, 4), (16, 56), None),
        ("rumba_flamenca", (100, 160), (4, 4), (24, 80), None),
        ("ska", (120, 180), (4, 4), (24, 80), None),
        ("rocksteady", (76, 110), (4, 4), (12, 40), None),
        ("reggae", (60, 96), (4, 4), (8, 32), None),
        ("dub", (60, 100), (4, 4), (4, 24), None),
        ("dancehall", (90, 120), (4, 4), (16, 64), None),
        ("afrobeat", (95, 130), (4, 4), (32, 128), None),
        ("highlife", (100, 150), (4, 4), (24, 80), None),
        ("soukous", (120, 180), (4, 4), (32, 128), None),
        ("amapiano", (108, 118), (4, 4), (16, 64), None),
        ("afro_cuban", (100, 180), (4, 4), (24, 96), None),
        ("raita", (90, 140), (4, 4), (24, 96), None),
        ("arabic_maqam", (70, 160), (4, 4), (12, 64), None),
        ("turkish_makam", (70, 160), (4, 4), (12, 64), None),
        ("persian_dastgah", (60, 140), (4, 4), (8, 48), None),
        ("hindustani", (50, 180), (4, 4), (8, 96), None),
        ("carnatic", (60, 180), (4, 4), (16, 96), None),
        ("bollywood", (80, 150), (4, 4), (24, 96), None),
        ("bhangra", (100, 160), (4, 4), (32, 128), None),
        ("gamelan", (50, 100), (4, 4), (16, 64), None),
        ("pentatonic_east_asian", (56, 120), (4, 4), (8, 40), None),
        ("andean", (80, 140), (4, 4), (16, 64), None),
    ];
    let rules = GenreLibrary::all();
    assert_eq!(rules.len(), TABLE.len(), "the registry size changed");
    for (index, (id, bpm, meter, density, swing)) in TABLE.iter().enumerate() {
        let rule = &rules[index];
        assert_eq!(rule.id, *id, "entry {index} changed id");
        assert_eq!(rule.default_bpm_range, *bpm, "{}: BPM range", rule.id);
        assert_eq!(rule.meter, *meter, "{}: meter", rule.id);
        assert_eq!(rule.note_density_hint, *density, "{}: density", rule.id);
        assert_eq!(
            rule.swing_permille().unwrap(),
            *swing,
            "{}: swing permille",
            rule.id
        );
    }
    // 位置敏感的自证：表里的 id 与 `genre_id_salt` 的输入一致，且**顺序**被钉住。
    let ids: Vec<&str> = TABLE.iter().map(|row| row.0).collect();
    assert_eq!(ids[0], "gregorian_chant");
    assert_eq!(ids[ids.len() - 1], rules[rules.len() - 1].id);
    // 求和与多重集两层仍然对账（三层读数互相交叉）。
    assert_eq!(TABLE.iter().map(|r| u32::from(r.1.0)).sum::<u32>(), 16814);
    assert_eq!(TABLE.iter().map(|r| u32::from(r.3.0)).sum::<u32>(), 3159);
    assert_eq!(TABLE.iter().filter(|r| r.4.is_some()).count(), 34);
}
