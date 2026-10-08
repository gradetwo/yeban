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
//! ④ **确定性** [ARCH-DET-001]：同一份字节解析两次必须 `Eq`；
//! ⑤ **`.mxl`（ZIP 容器）的代价与路线**：自造的 deflate 容器必须在 UTF-8 这一步就明确 `Err`，
//!    容器**自己**的 CRC-32 / 尺寸字段用来（**不解压**地）证明 `score.xml` 的内容与纯文本夹具
//!    逐字节相同，并钉住"复用 `yeban-model` 的 ZIP 读取器"这条路线今天**不通**。
//! ⑥ **`.mxl` 只读导入**（本票新增，`yeban_midi::mxl`）：容器解开后必须交出与纯文本**同一个**
//!    [`MusicXmlScore`]；根文件名只认 `META-INF/container.xml` 的 `<rootfile full-path>`（⛔ 不猜）；
//!    字段不符 / 压缩法不认识 / 加密 / ZIP64 / 两种炸弹（声明超界、实际膨胀超界）都明确 `Err`；
//!    容器层的变形（截断 / 翻转）不 panic。容器由**判据自己**拼 ⇒ 受测代码不是自己的裁判。
//!
//! ## 规范出处
//!
//! 规范**未定义** MusicXML（四份规范里 `musicxml` 的命中数为 **0**；测法见
//! `src/musicxml.rs` 的模块文档）。本文件的依据是工程裁决
//! `docs/ledger/integration-rulings-notes.md:14`（R1：不引入新依赖，手写 pull parser）。
//!
//! ## 本文件**没有**证明什么
//!
//! - ⛔ 不证明 `.mxl` 的**全部**形态可读：只覆盖 2 个已提交夹具（`score.xml` 分别是
//!   dynamic 与 fixed Huffman 块）与判据自造的容器（stored 条目、stored DEFLATE 块）。
//!   ZIP64 / 加密 / 非 deflate 压缩法 / data descriptor 的**接受**都**没有**判据
//!   —— 前三者的**拒绝**有判据，`data descriptor` 连判据都没有（见 `src/mxl.rs` 的边界 7）。
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
use yeban_midi::mxl::{MxlError, MxlLimits, parse_mxl, parse_mxl_with_limits};
use yeban_model::container::{ContainerError, ContainerLimits, read_container};

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
/// 自造夹具：把 [`HANDMADE_MVP`] 打包成 deflate ZIP（`.mxl`）。
/// 构造配方 / 字节 / SHA-256 见 `tests/fixtures/README.md` 第 6 节。
const HANDMADE_MXL: &[u8] = include_bytes!("fixtures/handmade_mvp_partwise.mxl");
/// 自造夹具：同上，但 `score.xml` 的 DEFLATE 流是**固定 Huffman**（`BTYPE=1`）块。
/// 构造配方 / 字节 / SHA-256 见 `tests/fixtures/README.md` 第 7 节。
const HANDMADE_MXL_FIXED: &[u8] =
    include_bytes!("fixtures/handmade_mvp_partwise_deflate_fixed.mxl");

/// 全部已提交的**纯文本**夹具（名字 + 字节）。
///
/// ⚠️ [`HANDMADE_MXL`] **不在**这里：本列表的读者之一 `parsing_is_deterministic_for_every_fixture`
/// 要求每个夹具都 `Ok`（`.mxl` 必然 `Err`）⇒ 它由 `mxl_*` 三条判据单独覆盖。
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

// ---------------------------------------------------------------------------
// `.mxl`（ZIP/deflate 容器）：代价与路线
//
// 本票**不**实现 `.mxl`（inflate 需依赖或有规模的手写解码器；
// `docs/ledger/integration-rulings-notes.md:37-38` 已裁决另立票）。
// 下面三条判据把"代价"与"路线"钉成会变红的字面值，并**不**引入任何依赖。
// ---------------------------------------------------------------------------

/// ZIP local file header 的字段（APPNOTE 4.3.7 的 local file header 布局）。
struct LocalEntry<'a> {
    /// 条目名（UTF-8，本夹具全是 ASCII）。
    name: &'a str,
    /// 压缩法：0 = stored，8 = deflate。
    method: u16,
    /// 未压缩内容的 CRC-32（IEEE 802.3，反射，多项式 `0xEDB88320`）。
    crc32: u32,
    /// 压缩后字节数（数据区长度）。
    compressed: u32,
    /// 未压缩字节数（**声明值**）。
    uncompressed: u32,
}

fn le16(bytes: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([bytes[at], bytes[at + 1]])
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// 按顺序读 local file header。**不解压**，因此本文件不需要 inflate 依赖。
///
/// 遇到非 `PK\x03\x04`（= central directory 或尾随字节）就停：本夹具没有 data descriptor，
/// 所以"下一条 local header"恰好跟着上一条的数据区。
fn local_entries(bytes: &[u8]) -> Vec<LocalEntry<'_>> {
    let mut entries = Vec::new();
    let mut pos = 0usize;
    while bytes.get(pos..pos + 4) == Some(b"PK\x03\x04") {
        if bytes.len() < pos + 30 {
            break;
        }
        let method = le16(bytes, pos + 8);
        let crc32 = le32(bytes, pos + 14);
        let compressed = le32(bytes, pos + 18);
        let uncompressed = le32(bytes, pos + 22);
        let name_len = usize::from(le16(bytes, pos + 26));
        let extra_len = usize::from(le16(bytes, pos + 28));
        let Some(raw_name) = bytes.get(pos + 30..pos + 30 + name_len) else {
            break;
        };
        let Ok(name) = core::str::from_utf8(raw_name) else {
            break;
        };
        entries.push(LocalEntry {
            name,
            method,
            crc32,
            compressed,
            uncompressed,
        });
        let next = pos + 30 + name_len + extra_len + compressed as usize;
        if next <= pos {
            break;
        }
        pos = next;
    }
    entries
}

/// CRC-32（IEEE 802.3，反射多项式 `0xEDB88320`）—— 容器**自己**的完整性字段。
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[test]
fn mxl_container_is_rejected_at_the_zip_header_not_by_a_parser_bug() {
    // 字面读数：本夹具第 0 个 local file header 长 52 字节（30 + 条目名 22 + 额外字段 0）；
    // 它的 CRC-32 字段（偏移 14..18）里 0xae 出现在偏移 17 ⇒ 不是合法 UTF-8
    // ⇒ parser 在**任何压缩字节之前**就停了（`score.xml` 的数据区从偏移 195 开始）。
    assert_eq!(HANDMADE_MXL.len(), 1435);
    assert_eq!(&HANDMADE_MXL[..4], b"PK\x03\x04");
    assert_eq!(HANDMADE_MXL[17], 0xae);
    assert_eq!(
        parse_musicxml(HANDMADE_MXL),
        Err(MusicXmlError::InvalidUtf8 { offset: 17 })
    );
    // 确定性 [ARCH-DET-001]：同一份字节两次结果相同。
    assert_eq!(parse_musicxml(HANDMADE_MXL), parse_musicxml(HANDMADE_MXL));
    // ⛔ 不主张这里是"最好的报错"：它只是**今天**的读数（容器层拦住了文本层）。
}

#[test]
fn the_existing_zip_reader_rejects_the_mxl_container_before_any_name_check() {
    // 路线 C 的实测：`yeban_model::container::read_container` 在**第一个**条目上就按压缩法拒绝
    // （`crates/yeban-model/src/container/zip.rs:486-490` 的 (3.1)），
    // 因此"复用 `.yeban` 容器的 ZIP 读取器来读 `.mxl`"这条路今天**不通**。
    //
    // ⚠️ 这钉的是**今天的读数**，不是承诺：若后来有票让 `read_zip` 支持 deflate，
    // 本判据会变红 —— 那时应当**改写**本判据（记录新路线），⛔ 不要删掉它。
    assert_eq!(
        read_container(HANDMADE_MXL, &ContainerLimits::default()),
        Err(ContainerError::UnsupportedCompression {
            index: 0,
            method: 8
        })
    );
}

#[test]
fn mxl_cost_is_pinned_by_the_container_fields_without_inflating() {
    let entries = local_entries(HANDMADE_MXL);
    assert_eq!(entries.len(), 2, "本夹具是 2 个条目的容器");
    let names: Vec<&str> = entries.iter().map(|entry| entry.name).collect();
    assert_eq!(names, vec!["META-INF/container.xml", "score.xml"]);
    // 2/2 都是 deflate ⇒ 只会读 `stored` 的读取器**一个条目**都读不出（路线 C 的根因）。
    assert!(entries.iter().all(|entry| entry.method == 8));

    // 条目 0：`META-INF/container.xml`（146 → 104 字节）。
    assert_eq!(entries[0].crc32, 0xae69_681f);
    assert_eq!((entries[0].uncompressed, entries[0].compressed), (146, 104));
    // 条目 1：`score.xml`（2716 → 1095 字节）。
    assert_eq!(entries[1].crc32, 0xcbb0_05a0);
    assert_eq!(
        (entries[1].uncompressed, entries[1].compressed),
        (2716, 1095)
    );

    // ⭐ 不解压也能证明"膨胀结果与纯文本夹具逐字节相同"：用容器**自己**的 CRC-32 与尺寸字段。
    // 于是新增的成本只可能是"容器 + inflate"，不可能来自格式差异。
    assert_eq!(HANDMADE_MVP.len(), entries[1].uncompressed as usize);
    assert_eq!(crc32(HANDMADE_MVP), entries[1].crc32);
    // 压缩比（无量纲）：2716 / 1095 = 2.48。
    assert!(entries[1].uncompressed > entries[1].compressed);
    // 同一份字节两次读出的字段相同（容器层也是确定性 [ARCH-DET-001]）。
    assert_eq!(
        local_entries(HANDMADE_MXL)
            .iter()
            .map(|entry| (entry.name, entry.method, entry.crc32))
            .collect::<Vec<_>>(),
        entries
            .iter()
            .map(|entry| (entry.name, entry.method, entry.crc32))
            .collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// `.mxl`（ZIP/deflate 容器）：**只读导入**（本票新增）
//
// 上面三条判据钉的是**代价与拒绝**（上一票 `a29d280`）。下面这些钉的是导入本身：
// `parse_mxl` 必须把容器解开，并交出与纯文本**同一个** `MusicXmlScore`。
// 容器要么是**独立生产者**（CPython `zipfile` / `zlib`）写的已提交夹具，
// 要么由**判据自己**拼（`build_zip`）⇒ 受测代码不是自己的裁判。
// ---------------------------------------------------------------------------

/// 判据侧的 ZIP 条目：**每个字段都可改** ⇒ 能造出"字段与数据不符"的容器。
struct ZipEntrySpec {
    name: Vec<u8>,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u32,
    uncompressed: u32,
    body: Vec<u8>,
}

impl ZipEntrySpec {
    /// 压缩法 **0**（stored）的条目：数据区就是载荷本身。
    fn stored(name: &str, payload: &[u8]) -> Self {
        Self {
            name: name.as_bytes().to_vec(),
            flags: 0,
            method: 0,
            crc: crc32(payload),
            compressed: payload.len() as u32,
            uncompressed: payload.len() as u32,
            body: payload.to_vec(),
        }
    }

    /// 压缩法 **8**（deflate）的条目，其 DEFLATE 流**只用一个 stored 块**（`BTYPE=00`）。
    ///
    /// ⛔ 这是判据侧的编码器：它不调用受测的解码器，因此"能读回来"不是同义反复。
    fn deflate_stored(name: &str, payload: &[u8]) -> Self {
        let body = deflate_stored_block(payload);
        Self {
            name: name.as_bytes().to_vec(),
            flags: 0,
            method: 8,
            crc: crc32(payload),
            compressed: body.len() as u32,
            uncompressed: payload.len() as u32,
            body,
        }
    }
}

/// 一个 stored 块（`BFINAL=1` / `BTYPE=00`）的 **raw DEFLATE** 流（RFC 1951 §3.2.4）。
fn deflate_stored_block(payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= 0xffff, "单个 stored 块的 LEN 是 16 位");
    let mut out = vec![0x01u8]; // bit0 = BFINAL = 1，bit1..2 = BTYPE = 00（低位先出）
    let length = payload.len() as u16;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(&(!length).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// 判据侧的 ZIP 写出器（APPNOTE 的 local file header / central directory / EOCD 三节）。
///
/// `total_override` 用来伪造 EOCD 里的条目数（ZIP64 标记 `0xFFFF` 就靠它）。
fn build_zip(entries: &[ZipEntrySpec], total_override: Option<u16>) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut central: Vec<u8> = Vec::new();
    for entry in entries {
        let offset = out.len() as u32;
        out.extend_from_slice(b"PK\x03\x04");
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&entry.flags.to_le_bytes());
        out.extend_from_slice(&entry.method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // mod time
        out.extend_from_slice(&0u16.to_le_bytes()); // mod date
        out.extend_from_slice(&entry.crc.to_le_bytes());
        out.extend_from_slice(&entry.compressed.to_le_bytes());
        out.extend_from_slice(&entry.uncompressed.to_le_bytes());
        out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra len
        out.extend_from_slice(&entry.name);
        out.extend_from_slice(&entry.body);

        central.extend_from_slice(b"PK\x01\x02");
        central.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central.extend_from_slice(&entry.flags.to_le_bytes());
        central.extend_from_slice(&entry.method.to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // mod time
        central.extend_from_slice(&0u16.to_le_bytes()); // mod date
        central.extend_from_slice(&entry.crc.to_le_bytes());
        central.extend_from_slice(&entry.compressed.to_le_bytes());
        central.extend_from_slice(&entry.uncompressed.to_le_bytes());
        central.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        central.extend_from_slice(&0u16.to_le_bytes()); // extra
        central.extend_from_slice(&0u16.to_le_bytes()); // comment
        central.extend_from_slice(&0u16.to_le_bytes()); // disk
        central.extend_from_slice(&0u16.to_le_bytes()); // internal attr
        central.extend_from_slice(&0u32.to_le_bytes()); // external attr
        central.extend_from_slice(&offset.to_le_bytes());
        central.extend_from_slice(&entry.name);
    }
    let cd_offset = out.len() as u32;
    let cd_size = central.len() as u32;
    out.extend_from_slice(&central);
    let total = total_override.unwrap_or(entries.len() as u16);
    out.extend_from_slice(b"PK\x05\x06");
    out.extend_from_slice(&0u16.to_le_bytes()); // disk
    out.extend_from_slice(&0u16.to_le_bytes()); // central directory disk
    out.extend_from_slice(&total.to_le_bytes()); // 本盘条目数
    out.extend_from_slice(&total.to_le_bytes()); // 总条目数
    out.extend_from_slice(&cd_size.to_le_bytes());
    out.extend_from_slice(&cd_offset.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment len
    out
}

/// 一份最小的 OPC `META-INF/container.xml`（形状与已提交夹具相同，只换 `full-path`）。
fn container_xml(full_path: &str) -> Vec<u8> {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<container>\n  <rootfiles>\n    \
         <rootfile full-path=\"{full_path}\">\n    </rootfile>\n  </rootfiles>\n</container>\n"
    )
    .into_bytes()
}

/// 与 [`local_entries`] 同一遍 local header 扫描，但给出每个条目的**数据区**。
///
/// 为什么单独一支（而不是改 `local_entries`）：上面三条 mxl 判据读的是**同一份夹具**，
/// 本票不动它们的形态。
fn local_bodies(bytes: &[u8]) -> Vec<&[u8]> {
    let mut bodies = Vec::new();
    let mut pos = 0usize;
    while bytes.get(pos..pos + 4) == Some(b"PK\x03\x04") {
        if bytes.len() < pos + 30 {
            break;
        }
        let compressed = le32(bytes, pos + 18) as usize;
        let name_len = usize::from(le16(bytes, pos + 26));
        let extra_len = usize::from(le16(bytes, pos + 28));
        let start = pos + 30 + name_len + extra_len;
        let Some(body) = bytes.get(start..start + compressed) else {
            break;
        };
        if start + compressed <= pos {
            break;
        }
        bodies.push(body);
        pos = start + compressed;
    }
    bodies
}

/// 给一个**还没有注释**的容器补一段 EOCD 注释（同时把 EOCD 的注释长度字段改成它的长度）。
fn append_eocd_comment(zip: &mut Vec<u8>, comment: &[u8]) {
    let eocd = zip.len() - 22;
    assert_eq!(
        &zip[eocd..eocd + 4],
        b"PK\x05\x06",
        "EOCD 必须正好在最后 22 字节"
    );
    assert_eq!(
        (le16(zip, eocd + 20), zip.len()),
        (0, eocd + 22),
        "本助手只接受还没有注释的容器"
    );
    zip[eocd + 20..eocd + 22].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    zip.extend_from_slice(comment);
}

/// 读一个 raw DEFLATE 流的**首块**类型：返回 `(BFINAL, BTYPE)`（RFC 1951 §3.1.1 的低位先出）。
fn first_deflate_block(body: &[u8]) -> (u8, u8) {
    let byte = body.first().copied().unwrap_or(0);
    (byte & 1, (byte >> 1) & 0b11)
}

#[test]
fn mxl_container_imports_the_same_score_as_the_plain_text() {
    let text = parse("handmade_mvp_partwise", HANDMADE_MVP);
    // 两个已提交夹具的 score.xml 分别走 **dynamic**（BTYPE=2，`1435` 字节那份）与
    // **fixed**（BTYPE=1，`1533` 字节那份）Huffman 表 ⇒ 两条码表路径都必须可用。
    let dynamic = parse_mxl(HANDMADE_MXL).expect("dynamic Huffman 容器必须可读");
    let fixed = parse_mxl(HANDMADE_MXL_FIXED).expect("fixed Huffman 容器必须可读");
    assert_eq!(dynamic, text, "容器导入与纯文本解析必须是同一个 score");
    assert_eq!(fixed, text, "换一张 Huffman 表不该改变 score");

    // 字面读数（单位写清）：divisions=4（每四分音符 4 单位）、音符条目数=4、部件 id=P1。
    assert_eq!(dynamic.divisions, 4);
    assert_eq!(dynamic.note_count(), 4);
    assert_eq!(dynamic.parts[0].id, "P1");
    assert_eq!(dynamic.parts[0].name, "Handmade MVP");
    // 确定性 [ARCH-DET-001]：同一份字节两次结果相同。
    assert_eq!(parse_mxl(HANDMADE_MXL), Ok(dynamic));
}

#[test]
fn mxl_import_readings_are_pinned_by_listing_the_containers() {
    // 两个夹具都是 **2** 个条目、**2/2** deflate；`score.xml` 的载荷就是纯文本夹具
    // （用容器**自己**的 CRC-32 与未压缩长度核对 ⇒ 不需要相信本模块的 inflate）。
    // `compressed` 是**实测**读数：1435 那份 1095 字节、1533 那份 1193 字节。
    for (name, bytes, compressed_score, btype) in [
        ("handmade_mvp_partwise.mxl", HANDMADE_MXL, 1095u32, 2u8),
        (
            "handmade_mvp_partwise_deflate_fixed.mxl",
            HANDMADE_MXL_FIXED,
            1193,
            1,
        ),
    ] {
        let entries = local_entries(bytes);
        let names: Vec<&str> = entries.iter().map(|entry| entry.name).collect();
        assert_eq!(names, vec!["META-INF/container.xml", "score.xml"], "{name}");
        assert!(entries.iter().all(|entry| entry.method == 8), "{name}");
        assert_eq!(entries[0].crc32, 0xae69_681f, "{name} 的 container.xml");
        assert_eq!(
            (entries[0].uncompressed, entries[0].compressed),
            (146, 104),
            "{name}"
        );
        assert_eq!(entries[1].crc32, crc32(HANDMADE_MVP), "{name} 的 score.xml");
        assert_eq!(
            (entries[1].uncompressed, entries[1].compressed),
            (2716, compressed_score),
            "{name}"
        );
        // 首块的 BTYPE 是**逐位**读出来的（不是本模块说的，是判据自己数的）。
        let bodies = local_bodies(bytes);
        assert_eq!(bodies.len(), 2, "{name}");
        assert_eq!(
            first_deflate_block(bodies[1]),
            (1, btype),
            "{name} 的 score.xml 首块"
        );
    }
}

#[test]
fn mxl_rootfile_path_is_followed_and_its_absence_is_explicit() {
    let text = parse("handmade_mvp_partwise", HANDMADE_MVP);

    // ① 根文件名**不是猜的**：container.xml 指向 `nested/part.xml`，条目也在那儿。
    let nested = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container_xml("nested/part.xml")),
            ZipEntrySpec::stored("nested/part.xml", HANDMADE_MVP),
        ],
        None,
    );
    assert_eq!(parse_mxl(&nested), Ok(text));

    // ② full-path 指向的条目不存在 ⇒ 明确 Err（⛔ 不回退成"唯一的 xml 条目就是根文件"）。
    let missing = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container_xml("absent.xml")),
            ZipEntrySpec::stored("score.xml", HANDMADE_MVP),
        ],
        None,
    );
    assert_eq!(
        parse_mxl(&missing),
        Err(MxlError::MissingRootFile {
            path: "absent.xml".to_owned()
        })
    );

    // ③ 没有 container.xml ⇒ 明确 Err（OPC 要求根文件由它指定）。
    let no_container = build_zip(&[ZipEntrySpec::stored("score.xml", HANDMADE_MVP)], None);
    assert_eq!(parse_mxl(&no_container), Err(MxlError::NoContainer));

    // ④ container.xml 里没有 <rootfile> ⇒ 明确 Err。
    let no_rootfile = build_zip(
        &[
            ZipEntrySpec::stored(
                "META-INF/container.xml",
                b"<container><rootfiles/></container>",
            ),
            ZipEntrySpec::stored("score.xml", HANDMADE_MVP),
        ],
        None,
    );
    assert_eq!(parse_mxl(&no_rootfile), Err(MxlError::NoRootFile));
}

#[test]
fn mxl_container_field_mismatches_are_rejected_by_name() {
    let container = container_xml("score.xml");
    let wrap = |spec: ZipEntrySpec| {
        build_zip(
            &[
                ZipEntrySpec::stored("META-INF/container.xml", &container),
                spec,
            ],
            None,
        )
    };

    // ① CRC-32 与数据不符 ⇒ 报**两个**读数（声明的与实际算出的）。
    let mut spec = ZipEntrySpec::stored("score.xml", HANDMADE_MVP);
    spec.crc ^= 1;
    assert_eq!(
        parse_mxl(&wrap(spec)),
        Err(MxlError::CrcMismatch {
            name: "score.xml".to_owned(),
            declared: crc32(HANDMADE_MVP) ^ 1,
            actual: crc32(HANDMADE_MVP),
        })
    );

    // ② 声明的未压缩长度与载荷不符。
    let mut spec = ZipEntrySpec::stored("score.xml", HANDMADE_MVP);
    spec.uncompressed -= 1;
    assert_eq!(
        parse_mxl(&wrap(spec)),
        Err(MxlError::SizeMismatch {
            name: "score.xml".to_owned(),
            declared: 2715,
            actual: 2716,
        })
    );

    // ③ 压缩法不是 0/8 ⇒ 点名压缩法（12 = bzip2）。
    let mut spec = ZipEntrySpec::stored("score.xml", HANDMADE_MVP);
    spec.method = 12;
    assert_eq!(
        parse_mxl(&wrap(spec)),
        Err(MxlError::UnsupportedCompression {
            name: "score.xml".to_owned(),
            method: 12,
        })
    );

    // ④ 加密位（general purpose flag 的 bit 0）⇒ 明确拒绝，⛔ 不尝试解密。
    let mut spec = ZipEntrySpec::stored("score.xml", HANDMADE_MVP);
    spec.flags = 0x0001;
    assert_eq!(
        parse_mxl(&wrap(spec)),
        Err(MxlError::Encrypted {
            name: "score.xml".to_owned()
        })
    );

    // ⑤ ZIP64 标记（EOCD 的条目数是 `0xFFFF`）⇒ 明确拒绝。
    let zip64 = build_zip(
        &[ZipEntrySpec::stored("score.xml", HANDMADE_MVP)],
        Some(0xffff),
    );
    assert_eq!(parse_mxl(&zip64), Err(MxlError::UnsupportedZip64));

    // ⑥ 不是容器 ⇒ NotZip（既不是 panic，也不是"文本层的 UTF-8 错误"）。
    assert_eq!(parse_mxl(HANDMADE_MVP), Err(MxlError::NotZip));
    assert_eq!(parse_mxl(b""), Err(MxlError::NotZip));

    // ⑦ 载荷是合法容器但**不是** MusicXML ⇒ 文本层的错误原样上传（⛔ 不吞掉）。
    let not_score = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container),
            ZipEntrySpec::stored("score.xml", b"<score-timewise></score-timewise>"),
        ],
        None,
    );
    assert_eq!(
        parse_mxl(&not_score),
        Err(MxlError::MusicXml(MusicXmlError::UnsupportedRoot {
            root: "score-timewise".to_owned()
        }))
    );

    // ⑧ 压缩法是 deflate 但流本身非法 ⇒ 在文本层之前就报，且报的是 inflate 的**字面**偏移
    //    （`0x07` 的低 3 位是 `BFINAL=1` / `BTYPE=11` ⇒ 未定义的块类型）。
    let broken_stream = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container),
            ZipEntrySpec {
                name: b"score.xml".to_vec(),
                flags: 0,
                method: 8,
                crc: 0,
                compressed: 1,
                uncompressed: 1,
                body: vec![0x07],
            },
        ],
        None,
    );
    assert_eq!(
        parse_mxl(&broken_stream),
        Err(MxlError::InvalidDeflate {
            offset: 1,
            detail: "块类型 3 未定义（RFC 1951 §3.2.3）",
        })
    );
}

#[test]
fn mxl_limits_stop_both_declared_and_actual_blowups() {
    let container = container_xml("score.xml");

    // ① 声明的未压缩长度超界 ⇒ **解压前**拒绝（第一项就是 container.xml 的 146 > 64）。
    let declared = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container),
            ZipEntrySpec::stored("score.xml", HANDMADE_MVP),
        ],
        None,
    );
    let tiny = MxlLimits {
        max_entry_bytes: 64,
        ..MxlLimits::default()
    };
    assert_eq!(
        parse_mxl_with_limits(&declared, &tiny),
        Err(MxlError::LimitExceeded {
            limit: "entry_bytes",
            value: 146,
            max: 64,
        })
    );

    // ② 声明**撒谎**：score.xml 声明 10 字节，DEFLATE 流实际膨胀到 2716 字节。
    //    上界（1024）在**每次写入前**检查 ⇒ 在上界处截停，⛔ 不是先把 2716 字节全读出来。
    let mut lying = ZipEntrySpec::deflate_stored("score.xml", HANDMADE_MVP);
    lying.uncompressed = 10;
    let bomb = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container),
            lying,
        ],
        None,
    );
    assert_eq!(
        parse_mxl_with_limits(
            &bomb,
            &MxlLimits {
                max_entry_bytes: 1024,
                ..MxlLimits::default()
            }
        ),
        Err(MxlError::InflatedTooLarge {
            name: "score.xml".to_owned(),
            max: 1024,
        })
    );

    // ③ 同一条流换个更大的上界就能读 ⇒ ②拒绝的是**上界**，不是流本身（对照臂）。
    assert_eq!(
        parse_mxl_with_limits(
            &bomb,
            &MxlLimits {
                max_entry_bytes: 4096,
                ..MxlLimits::default()
            }
        ),
        Err(MxlError::SizeMismatch {
            name: "score.xml".to_owned(),
            declared: 10,
            actual: 2716,
        })
    );

    // ④ 条目名长度上界（`META-INF/container.xml` 是 22 字节、上界 4）⇒ 在**读名字之前**拒绝。
    assert_eq!(
        parse_mxl_with_limits(
            &declared,
            &MxlLimits {
                max_name_bytes: 4,
                ..MxlLimits::default()
            }
        ),
        Err(MxlError::LimitExceeded {
            limit: "name_bytes",
            value: 22,
            max: 4,
        })
    );

    // ⑤ 条目数上界（中央目录声明 3 个、上界 2）⇒ 在**读目录之前**拒绝。
    let three = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container),
            ZipEntrySpec::stored("score.xml", HANDMADE_MVP),
            ZipEntrySpec::stored("extra.xml", b"<x/>"),
        ],
        None,
    );
    assert_eq!(
        parse_mxl_with_limits(
            &three,
            &MxlLimits {
                max_entries: 2,
                ..MxlLimits::default()
            }
        ),
        Err(MxlError::LimitExceeded {
            limit: "entries",
            value: 3,
            max: 2,
        })
    );
}

#[test]
fn mxl_container_fuzz_never_panics() {
    // 3 个容器：2 个已提交夹具 + 1 个判据自造（stored 条目 + stored DEFLATE 块）。
    // 对每个做**截断 / 翻转**，只允许 Ok 或 Err。
    let built = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container_xml("score.xml")),
            ZipEntrySpec::deflate_stored("score.xml", HANDMADE_MVP),
        ],
        None,
    );
    let containers: [(&str, &[u8]); 3] = [
        ("handmade_mvp_partwise.mxl", HANDMADE_MXL),
        (
            "handmade_mvp_partwise_deflate_fixed.mxl",
            HANDMADE_MXL_FIXED,
        ),
        ("built_stored_and_deflate_stored", &built),
    ];
    let mut runs = 0usize;
    for (name, bytes) in containers {
        assert!(
            parse_mxl(bytes).is_ok(),
            "{name} 在变形之前就必须是可读的（否则本判据没有意义）"
        );
        for cut in 0..bytes.len() {
            let _ = parse_mxl(&bytes[..cut]);
            runs += 1;
        }
        for index in (0..bytes.len()).step_by(3) {
            for replacement in [0x00u8, 0xff, b'P', b'K'] {
                let mut copy = bytes.to_vec();
                copy[index] = replacement;
                let _ = parse_mxl(&copy);
                runs += 1;
            }
        }
    }
    // 判据本身是"没 panic"；这个数字让"到底跑了多少次"可复核。
    println!("mxl_container_fuzz_never_panics: runs={runs}");
    assert!(runs >= 5_000, "探针只跑了 {runs} 次，样本太少");
}

// ---------------------------------------------------------------------------
// `<forward>`: **今天被忽略的代价**（钉住读数，不是承诺）
//
// 本票**没有**实现 `forward`。原因不是"做不到"（修法是 `on_end("forward")` 里一行：
// 把 `<duration>` 换算成 tick 后前进 cursor，与 `backup` 完全对称），而是它**必然**
// 改掉一条**既有判据**的期望值：`crates/yeban-midi/src/musicxml.rs` 的单元测试
// `unsupported_elements_are_registered_separately` 断言
// `unsupported_elements.get("forward") == Some(&1)`（那个文件里现位于第 1505 行）
// —— 实现了就不再登记。本票纪律是"不许改既有判据期望值（要改 ⇒ 停下报告）"
// ⇒ 本票只把**代价**钉成读数，把红行交给下一票。
//
// ⛔ 公开语料里没有 `forward` 用例：已提交的 5 个 W3C 夹具与 2 个自造夹具里
// `forward` 的出现次数都是 **0**（测法：对 `tests/fixtures/*.musicxml` 逐文件数
// `<forward>` 的出现次数，全部 0）⇒ 只能自造（本票纪律：夹具只许自造或公有领域）。
// ---------------------------------------------------------------------------

/// 自造夹具：`<forward>` 的三种形状（带 `duration` / `duration=0` / 没有 `duration`）。
///
/// 构造配方就在本函数里，没有第二个来源。`divisions=4` ⇒ 1 unit = 240 tick。
fn forward_fixture() -> Vec<u8> {
    concat!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
        "<!DOCTYPE score-partwise PUBLIC \"-//Recordare//DTD MusicXML 4.0 Partwise//EN\" ",
        "\"http://www.musicxml.org/dtds/partwise.dtd\">\n",
        "<score-partwise version=\"4.0\">\n",
        "  <part-list><score-part id=\"P1\"><part-name>Forward</part-name></score-part></part-list>\n",
        "  <part id=\"P1\"><measure number=\"1\">\n",
        "    <attributes><divisions>4</divisions></attributes>\n",
        "    <note><pitch><step>C</step><octave>4</octave></pitch><duration>4</duration>",
        "<voice>1</voice></note>\n",
        "    <backup><duration>4</duration></backup>\n",
        "    <forward><duration>8</duration><voice>2</voice></forward>\n",
        "    <note><pitch><step>E</step><octave>4</octave></pitch><duration>4</duration>",
        "<voice>2</voice></note>\n",
        "    <forward><duration>0</duration></forward>\n",
        "    <forward/>\n",
        "    <note><pitch><step>G</step><octave>4</octave></pitch><duration>4</duration>",
        "<voice>2</voice></note>\n",
        "  </measure></part>\n",
        "</score-partwise>\n",
    )
    .as_bytes()
    .to_vec()
}

/// 判据（⚠️ **钉住今天的读数，不是承诺**）: `<forward>` 被忽略 ⇒ 它之后的每个
/// `start_tick` 都**少了 `forward` 的那一段**。
///
/// ## 量什么（单位 = tick，960 PPQ）
///
/// 夹具 `divisions=4` ⇒ 1 unit = 240 tick。`backup` 把 cursor 从 4 unit 送回 0；
/// 接着一个 `forward` 声明 8 unit（= 1920 tick）；再一个四分音符。
/// 读数 = 那个音符的 `start_tick`。
///
/// - **今天**（`forward` 未实现）: `1920 → 0`、`2880 → 960`
///   （两个空 `forward` 不声明 duration，本来就不该移动 ⇒ 它们的读数不变）。
/// - **实现之后**（cursor 前进）应当是: `(0, 1920, 2880)`。
///
/// 注入证据（本票实测，两个方向都跑过）: 把本票写好的 `on_end("forward")` 前进逻辑
/// 放进 `src/musicxml.rs` 后，本条的字面红行正是
/// `left: [(0, 60, 1, 1, 960), (0, 64, 2, 1, 960), (960, 67, 2, 1, 960)]` 对
/// `right: [(0, 60, 1, 1, 960), (1920, 64, 2, 1, 960), (2880, 67, 2, 1, 960)]`。
///
/// ⚠️ 本条会在 `forward` 被实现时**变红**。那时**应当改写**本条（改成断言正确的
/// tick，并把 `unsupported_elements` 的断言去掉），⛔ 不要删掉它。
#[test]
fn forward_is_ignored_and_shifts_every_later_tick() {
    let bytes = forward_fixture();
    let score = parse("forward_fixture", &bytes);
    assert_eq!(score.divisions, 4, "夹具声明 divisions=4");
    let actual: Vec<(u64, u8, u16, u16, u64)> = score.parts[0].notes.iter().map(literal).collect();
    //（`<staff>` 在夹具里没有出现 ⇒ 三个音符的 staff 都是缺省值 1。）
    assert_eq!(
        actual,
        vec![
            (0, 60, 1, 1, 960),   // C4：四分音符
            (0, 64, 2, 1, 960),   // E4：今天在 tick 0 —— forward 的 8 unit 被丢掉
            (960, 67, 2, 1, 960), // G4：今天在 960 —— 两个空 forward 不动 cursor
        ],
        "已知缺陷: <forward> 被忽略 ⇒ 其后 tick 少了 forward 的那一段"
    );
    // `forward` 今天登记在"已知未实现"表里（3 次出现 = 夹具里的 3 个 `<forward>`）。
    assert_eq!(score.unsupported_elements.get("forward"), Some(&3));
    // 夹具里没有白名单外的元素。
    assert!(
        score.ignored_elements.is_empty(),
        "白名单外的元素: {:?}",
        score.ignored_elements
    );
    assert_eq!(score.tick_range(), Some((0, 1920)));
    // 确定性 [ARCH-DET-001]：同一份字节两次解析相同。
    assert_eq!(parse_musicxml(&bytes), Ok(score));
}

#[test]
fn mxl_eocd_scan_honours_the_comment_length() {
    // EOCD 的注释是**任意字节**（APPNOTE 4.3.16）⇒ 注释里可以逐字节出现 `PK\x05\x06`。
    // 一个只认"从文件尾往前第一个签名"的扫描会被注释里的**假** EOCD 顶替。
    // 本条钉住 `find_eocd` 的 `pos + 22 + comment == 文件长度` 那一句。
    let mut bytes = build_zip(
        &[
            ZipEntrySpec::stored("META-INF/container.xml", &container_xml("score.xml")),
            ZipEntrySpec::stored("score.xml", HANDMADE_MVP),
        ],
        None,
    );
    let expected = parse_mxl(&bytes).expect("加注释之前必须可读");
    // 24 字节的注释：4 字节假签名 + 20 个 0 ⇒ 假 EOCD 的条目数 / 目录尺寸 / 目录偏移全是 0。
    let mut comment = b"PK\x05\x06".to_vec();
    comment.extend_from_slice(&[0u8; 20]);
    append_eocd_comment(&mut bytes, &comment);
    assert_eq!(
        parse_mxl(&bytes),
        Ok(expected),
        "注释里的假 EOCD 不该顶替真的 EOCD"
    );
}
