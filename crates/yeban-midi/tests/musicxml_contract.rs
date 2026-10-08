//! `yeban-midi` MusicXML 只读导入 MVP 的**集成判据**（只经公开 API）。
//!
//! ## 这个文件补的是哪个缺口
//!
//! `src/musicxml.rs` 是本票新增的模块。它的单元测试住在模块内部（可以碰私有函数），
//! 因此本文件从**调用方视角**（`yeban_midi::musicxml::*`）再钉一层：
//!
//! ① **自造夹具的逐字段字面值**：音高 / 时值 / **tick**（单位：**tick**，960 PPQ，
//!    1 四分音符 = 960 tick）全部写死，parser 丢音符、丢 tie 合并、算错换算都会变红；
//! ② **外部夹具（W3C CG 测试套件，MIT）的读数**：来源 / 许可 / SHA-256 见
//!    `tests/fixtures/README.md` 第 5 节；
//! ③ **任意字节零 panic**：对夹具字节做截断 / 翻转 / 插入，只允许 `Ok` 或 `Err`；
//! ④ **确定性** [ARCH-DET-001]：同一份字节解析两次必须 `Eq`。
//!
//! ## 规范出处
//!
//! 规范**未定义** MusicXML（四份规范里 `musicxml` 的命中数为 **0**；测法见
//! `src/musicxml.rs` 的模块文档）。本文件的依据是工程裁决
//! `docs/ledger/integration-rulings-notes.md:14`（R1：不引入新依赖，手写 pull parser）。
//!
//! ## 本文件**没有**证明什么
//!
//! - ⛔ 不证明 `.mxl`（ZIP）可读：那**没有实现**（见模块文档的未实现清单）。
//! - ⛔ 不证明导出、引擎接线、界面可用：都不存在。
//! - ⛔ 不证明完整 MusicXML 4.0 语义：`forward` / `grace` / `unpitched` / `transpose`
//!   只证明"被登记为未实现"，不证明语义正确。
//! - ⛔ 不证明"任意字节不 panic"是**穷尽**的：它是**探针**（对本目录夹具的变形），
//!   不是形式化证明。跑的次数在测试里数出来。

use std::collections::BTreeMap;

use yeban_midi::midi::{DEFAULT_PPQ, MidiTempo};
use yeban_midi::musicxml::{
    DEFAULT_VELOCITY, MAX_DEPTH, MusicXmlError, MusicXmlNote, MusicXmlScore, parse_musicxml,
};

/// 自造夹具：覆盖六项要求（单声部 / 和弦 / 延音 / backup 多声部 / divisions≠1 / 速度）。
const HANDMADE_MVP: &[u8] = include_bytes!("fixtures/handmade_mvp_partwise.musicxml");
/// 自造夹具：覆盖容错路径（DOCTYPE 内部子集 / 注释 / PI / 实体 / 未知标签 / 非 ASCII）。
const HANDMADE_TOLERANCE: &[u8] = include_bytes!("fixtures/handmade_tolerance.musicxml");
/// W3C CG 测试套件 `21a-Chord-Basic.musicxml`（MIT）。
const W3C_21A: &[u8] = include_bytes!("fixtures/w3c_21a_chord_basic.musicxml");
/// W3C CG 测试套件 `33b-Spanners-Tie.musicxml`（MIT）。
const W3C_33B: &[u8] = include_bytes!("fixtures/w3c_33b_spanners_tie.musicxml");
/// W3C CG 测试套件 `43a-PianoStaff.musicxml`（MIT）。
const W3C_43A: &[u8] = include_bytes!("fixtures/w3c_43a_piano_staff.musicxml");
/// W3C CG 测试套件 `03e-Rhythm-No-Divisions.musicxml`（MIT）。
const W3C_03E: &[u8] = include_bytes!("fixtures/w3c_03e_no_divisions.musicxml");
/// W3C CG 测试套件 `41h-TooManyParts.musicxml`（MIT）。
const W3C_41H: &[u8] = include_bytes!("fixtures/w3c_41h_multi_part.musicxml");

/// 全部已提交夹具（名字 + 字节）。
const FIXTURES: &[(&str, &[u8])] = &[
    ("handmade_mvp_partwise", HANDMADE_MVP),
    ("handmade_tolerance", HANDMADE_TOLERANCE),
    ("w3c_21a_chord_basic", W3C_21A),
    ("w3c_33b_spanners_tie", W3C_33B),
    ("w3c_43a_piano_staff", W3C_43A),
    ("w3c_03e_no_divisions", W3C_03E),
    ("w3c_41h_multi_part", W3C_41H),
];

fn parse(name: &str, bytes: &[u8]) -> MusicXmlScore {
    parse_musicxml(bytes).unwrap_or_else(|error| panic!("{name} 解析失败: {error}"))
}

/// 一个音符的**字面**读数 `(start_tick, key, voice, staff, duration_ticks)`。
fn literal(note: &MusicXmlNote) -> (u64, u8, u16, u16, u64) {
    (
        note.start_tick,
        note.key,
        note.voice,
        note.staff,
        note.duration_ticks,
    )
}

#[test]
fn handmade_mvp_every_field_is_a_literal() {
    let score = parse("handmade_mvp_partwise", HANDMADE_MVP);

    assert_eq!(score.divisions, 4, "夹具声明 divisions=4");
    assert_eq!(score.ppq, DEFAULT_PPQ, "输出恒为 960 PPQ");
    assert_eq!(score.parts.len(), 1);
    assert_eq!(score.parts[0].id, "P1");
    assert_eq!(score.parts[0].name, "Handmade MVP");

    // tick 全为字面值：divisions=4 ⇒ 1 unit = 240 tick。
    let expected: Vec<(u64, u8, u16, u16, u64)> = vec![
        (0, 48, 2, 2, 1920),  // C3 = (3+1)*12：voice 2 / staff 2，在 backup 之后
        (0, 60, 1, 1, 960),   // C4：四分音符
        (0, 64, 1, 1, 960),   // E4：<chord/> ⇒ 与 C4 同起点
        (960, 67, 1, 1, 960), // G4：两个八分音符被 <tie> 合成为一个四分音符
    ];
    let actual: Vec<(u64, u8, u16, u16, u64)> = score.parts[0].notes.iter().map(literal).collect();
    assert_eq!(actual, expected);
    assert_eq!(score.note_count(), 4);
    assert_eq!(score.tick_range(), Some((0, 1920)));
    assert!(
        score.parts[0]
            .notes
            .iter()
            .all(|n| n.velocity == DEFAULT_VELOCITY)
    );

    // 速度与拍号：两条记录，都在 tick 0。排序键 `Option` 的 None 在前。
    assert_eq!(
        score.tempos,
        vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: None,
                numerator: Some(3),
                denominator_pow2: Some(3),
            },
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: Some(666_667), // round(60_000_000 / 90)
                numerator: None,
                denominator_pow2: None,
            },
        ]
    );
}

#[test]
fn handmade_tolerance_reads_entities_and_registers_unknown_elements() {
    let score = parse("handmade_tolerance", HANDMADE_TOLERANCE);

    // divisions 缺省 ⇒ 1（出处：W3C 用例 03e 的自述 + XSD 的 minOccurs="0"）。
    assert_eq!(score.divisions, 1);
    assert_eq!(score.parts[0].name, "Handmade & tolerance — 夜半");
    // D♭4 = (4+1)*12 + 2 + (-1) = 61；两个全音符被 <tied>（在未知容器 <notations> 内）合成 2 个 unit。
    assert_eq!(
        score.parts[0].notes,
        vec![MusicXmlNote {
            key: 61,
            velocity: DEFAULT_VELOCITY,
            start_tick: 0,
            duration_ticks: 1920,
            voice: 1,
            staff: 1,
        }]
    );
    assert_eq!(score.parts[0].notes[0].end_tick(), 1920);

    // 未知元素逐一登记（出现次数）。
    let expected: BTreeMap<String, u64> = [
        ("left-margin", 1),
        ("print", 1),
        ("slur", 1),
        ("system-layout", 1),
        ("system-margins", 1),
        ("type", 2),
    ]
    .into_iter()
    .map(|(name, count)| (name.to_owned(), count))
    .collect();
    assert_eq!(score.ignored_elements, expected);
    assert!(score.unsupported_elements.is_empty());
}

#[test]
fn w3c_chord_fixture_reads_two_notes_and_a_rest() {
    // 夹具自述：A chord consisting of two quarter notes followed by a quarter rest.
    let score = parse("w3c_21a", W3C_21A);
    assert_eq!(score.divisions, 960);
    let actual: Vec<(u64, u8, u16, u16, u64)> = score.parts[0].notes.iter().map(literal).collect();
    // 休止符不产生音符，但推进 cursor（因此两个音符都在 tick 0）。
    assert_eq!(actual, vec![(0, 65, 1, 1, 960), (0, 69, 1, 1, 960)]);
    assert_eq!(score.note_count(), 2);
    assert_eq!(score.tick_range(), Some((0, 960)));
}

#[test]
fn w3c_tie_fixture_merges_two_whole_notes_into_one() {
    // 夹具自述：Two whole notes with a tie inbetween.
    let score = parse("w3c_33b", W3C_33B);
    assert_eq!(score.divisions, 1);
    assert_eq!(
        score.parts[0].notes,
        vec![MusicXmlNote {
            key: 65, // F4
            velocity: DEFAULT_VELOCITY,
            start_tick: 0,
            duration_ticks: 7680, // 4 + 4 个 unit，divisions=1 ⇒ 8 * 960
            voice: 1,
            staff: 1,
        }]
    );
    assert_eq!(score.tick_range(), Some((0, 7680)));
}

#[test]
fn w3c_piano_staff_fixture_reads_two_voices_after_backup() {
    // 夹具自述：A simple piano staff, i.e., two voices, each on a separate staff.
    let score = parse("w3c_43a", W3C_43A);
    assert_eq!(score.divisions, 96);
    let actual: Vec<(u64, u8, u16, u16, u64)> = score.parts[0].notes.iter().map(literal).collect();
    // 384 unit / 96 = 4 个四分音符 = 3840 tick；backup 把 cursor 送回 tick 0。
    assert_eq!(actual, vec![(0, 47, 2, 2, 3840), (0, 65, 1, 1, 3840)]);
    assert_eq!(score.note_count(), 2);
}

#[test]
fn w3c_no_divisions_fixture_defaults_to_one() {
    // 夹具自述：No <divisions> element. The generally agreed default value is 1.
    let score = parse("w3c_03e", W3C_03E);
    assert_eq!(score.divisions, 1);
    assert_eq!(
        score.parts[0].notes,
        vec![MusicXmlNote {
            key: 60, // C4
            velocity: DEFAULT_VELOCITY,
            start_tick: 0,
            duration_ticks: 3840, // 4 个 unit
            voice: 1,
            staff: 1,
        }]
    );
}

#[test]
fn w3c_multi_part_fixture_keeps_unlisted_parts_with_an_empty_name() {
    // 夹具自述：two more <part> elements than the <part-list> section contains.
    let score = parse("w3c_41h", W3C_41H);
    let ids: Vec<&str> = score.parts.iter().map(|part| part.id.as_str()).collect();
    let names: Vec<&str> = score.parts.iter().map(|part| part.name.as_str()).collect();
    assert_eq!(ids, vec!["P1", "P3", "P4"]);
    // P3 / P4 未登记在 <part-list> ⇒ 名字如实为空字符串，不臆造。
    assert_eq!(names, vec!["MusicXML Part", "", ""]);
    assert_eq!(score.note_count(), 0, "夹具里三个 part 全是休止符");
    assert_eq!(score.tick_range(), None);
}

#[test]
fn parsing_is_deterministic_for_every_fixture() {
    for (name, bytes) in FIXTURES {
        let first = parse(name, bytes);
        let second = parse(name, bytes);
        assert_eq!(first, second, "{name} 两次解析结果不同");
    }
}

#[test]
fn mxl_zip_bytes_are_rejected_without_panicking() {
    // `.mxl` 是 ZIP（`PK\x03\x04`）。本 MVP 只吃纯文本 ⇒ 必须是明确的 Err。
    // 含非 UTF-8 字节的 ZIP 前缀 ⇒ 明确 InvalidUtf8（真 .mxl 文件的读数见本票报告）。
    assert_eq!(
        parse_musicxml(&[0x50, 0x4b, 0x03, 0x04, 0xff, 0x00]),
        Err(MusicXmlError::InvalidUtf8 { offset: 4 })
    );
    // 纯 ASCII 的 ZIP 前缀没有非 UTF-8 字节，但也没有任何元素 ⇒ Empty。
    assert_eq!(
        parse_musicxml(&[0x50, 0x4b, 0x03, 0x04, 0x14, 0x00, 0x00, 0x00]),
        Err(MusicXmlError::Empty)
    );
}

#[test]
fn structural_errors_are_explicit() {
    assert_eq!(parse_musicxml(b""), Err(MusicXmlError::Empty));
    assert_eq!(
        parse_musicxml(b"<score-timewise></score-timewise>"),
        Err(MusicXmlError::UnsupportedRoot {
            root: "score-timewise".to_owned()
        })
    );
    assert!(matches!(
        parse_musicxml(b"<score-partwise><part-list>"),
        Err(MusicXmlError::Malformed { .. })
    ));
    assert!(matches!(
        parse_musicxml(b"<score-partwise></part></score-partwise>"),
        Err(MusicXmlError::Malformed { .. })
    ));
    assert!(matches!(
        parse_musicxml(b"<score-partwise><!-- never closed"),
        Err(MusicXmlError::Malformed { .. })
    ));
}

#[test]
fn arbitrary_bytes_never_panic() {
    // 数的是"跑了几次 parse"（单位：次调用）。任何一次 panic 都会让本测试失败。
    let mut runs: usize = 0;
    for (_, bytes) in FIXTURES {
        // ① 逐步截断：每个前缀都是一个可能非法但必须不 panic 的输入。
        for cut in (0..bytes.len()).step_by(5) {
            let _ = parse_musicxml(&bytes[..cut]);
            runs += 1;
        }
        // ② 逐字节翻转：把某处改成 0x00 / '&' / '<' / 0xff。
        let mut copy = bytes.to_vec();
        for index in (0..bytes.len()).step_by(13) {
            for replacement in [0x00u8, b'&', b'<', 0xff] {
                copy[index] = replacement;
                let _ = parse_musicxml(&copy);
                runs += 1;
            }
            copy[index] = bytes[index];
        }
        // ③ 插入一个 `<`：制造新的（通常未闭合的）标签边界。
        let mut inserted = bytes.to_vec();
        for index in (0..bytes.len()).step_by(97) {
            inserted.insert(index, b'<');
            let _ = parse_musicxml(&inserted);
            runs += 1;
            inserted.remove(index);
        }
    }
    // ④ 病态输入（含深嵌套上限与算术边界的邻居）。
    let deep: String = format!(
        "<score-partwise>{}{}",
        "<x>".repeat(MAX_DEPTH + 4),
        "</x>".repeat(MAX_DEPTH + 4)
    );
    let pathological: Vec<&[u8]> = vec![
        b"",
        b" ",
        b"<",
        b">",
        b"<>",
        b"</>",
        b"</a>",
        b"<a",
        b"<!",
        b"<!--",
        b"<?",
        b"<![CDATA[",
        b"<score-partwise",
        b"<score-partwise>",
        b"&amp;",
        b"\xff\xfe\x00\x00",
        b"\xef\xbb\xbf<score-partwise></score-partwise>",
        deep.as_bytes(),
        b"<score-partwise><part id='P'><measure><attributes><divisions>-1</divisions>\
          </attributes></measure></part></score-partwise>",
        b"<score-partwise><part id='P'><measure><note><pitch><step>H</step><octave>4</octave>\
          </pitch><duration>1</duration></note></measure></part></score-partwise>",
        b"<score-partwise><part id='P'><measure><note><pitch><step>C</step><octave>4</octave>\
          </pitch><duration>1</duration><voice>99999</voice></note></measure></part>\
          </score-partwise>",
        b"<score-partwise><part id='P'><measure><time><beats>4</beats><beat-type>3</beat-type>\
          </time></measure></part></score-partwise>",
        b"<score-partwise><part id='P'><measure><direction><sound tempo='0'/></direction>\
          </measure></part></score-partwise>",
        b"<score-partwise><part id='P'><measure><attributes><divisions>1</divisions></attributes>\
          <note><pitch><step>C</step><octave>4</octave></pitch><duration>9223372036854775808\
          </duration></note></measure></part></score-partwise>",
    ];
    for bytes in pathological {
        let _ = parse_musicxml(bytes);
        runs += 1;
    }

    // 判据本身是"没 panic"；这个数字让"到底跑了多少次"可复核。
    println!("arbitrary_bytes_never_panic: runs={runs}");
    assert!(runs >= 5_000, "探针只跑了 {runs} 次，样本太少");
}
