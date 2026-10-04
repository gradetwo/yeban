//! 畸形输入 / 显式上限 / 确定性判据。
//!
//! 这是 `MUST-GATE-011`（cargo-fuzz 千万次变异零崩溃）在**本机可跑**的那一层：
//! 一组固定畸形样本 + 确定性伪随机字节流（含结构化字母表偏置），断言解析器
//! **只返回 `Result`，永不 panic**。真正的 cargo-fuzz 目标在 `fuzz/`，本机不跑（见 notes）。
//!
//! 判据纪律：这些测试都曾经在「故意改坏实现」时变红，见
//! `docs/ledger/sfz-core-notes.md` 的「注入记录」表。

use std::fs;

use yeban_sfz::{IncludeResolver, ParseLimits, SfzError, parse_sources, parse_text};

mod support;

use support::TempDir;

/// 固定的畸形样本（人工挑选，覆盖每条分支的边界）。
const MALFORMED_SAMPLES: &[&str] = &[
    "",
    "\0",
    "<",
    "<>",
    "<region",
    "<region>",
    "<region>sample=",
    "<region>sample",
    "<region>sample=a.wav lokey=",
    "<region>sample=a.wav lokey=abc",
    "<region>sample=a.wav lokey=99999999999999999999",
    "<region>sample=a.wav lovel=-1",
    "<region>sample=a.wav lovel=300",
    "<region>sample=a.wav volume=NaN",
    "<region>sample=a.wav volume=inf",
    "<region>sample=a.wav volume=-inf",
    "<region>sample=a.wav pan=1e999",
    "<region>sample=a.wav loop_mode=bogus",
    "<region>sample=a.wav key=H9",
    "<region>sample=a.wav key=9999999999999",
    "<region>sample=a.wav pitch_keycenter=99999999999",
    "<region>sample=a.wav locc1=999",
    "<region>sample=a.wav locc=",
    "<region>sample=a.wav hicc999999=1",
    "<region>sample=a.wav seq_length=0",
    "<region>sample=a.wav seq_position=0",
    "#define",
    "#define $",
    "#define $A",
    "#define $A $A",
    "#define $A $B\n#define $B $A",
    "#include",
    "#include ",
    "#include x",
    "#include \"",
    "#include \"\"",
    "#include \"..\"",
    "#include \"../../../../etc/passwd\"",
    "#include \"/etc/passwd\"",
    "#include \"a\\..\\b.sfz\"",
    "#unknown-directive",
    "#",
    "///",
    "// only a comment",
    "<global>\n\n\n",
    "<group>",
    "<region>sample=a.wav\n<region>\n<region>sample=b.wav",
    "<region>sample=a.wav key=-1",
    "<region>sample=a.wav<region>sample=b.wav",
    "<region>sample=\"quoted.wav\" key=36",
    "<region>sample=a b c.wav key=36",
    "<region>\tsample=a.wav\tkey=36",
    "\u{feff}<region>sample=a.wav",
    "<region>sample=日本語.wav",
    "<region>sample=ë.wav key=c#4",
    "<region>sample=a.wav ",
    "<region>sample=a.wav //",
    "<region>sample=a.wav // <region>sample=b.wav",
    "<curve>",
    "<master>key=36",
    "<effect>",
    "<midi>",
    "<sample>",
    "sample=a.wav",
    "=a.wav",
    "key=36",
    "<region>sample=a.wav key=36 key=48",
];

#[test]
fn fixed_malformed_corpus_never_panics() {
    let limits = ParseLimits::default();
    for sample in MALFORMED_SAMPLES {
        // 只关心「不 panic」；Ok / Err 都合法。
        let first = parse_text(sample, &limits);
        let second = parse_text(sample, &limits);
        // 同一个输入必须得到同一结果（确定性）。
        assert_eq!(
            format!("{first:?}"),
            format!("{second:?}"),
            "non-deterministic verdict for {sample:?}"
        );
    }
}

/// 确定性 LCG（xorshift64），不引入任何随机数依赖。
struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        let mut value = self.0;
        value ^= value << 13;
        value ^= value >> 7;
        value ^= value << 17;
        self.0 = value;
        value
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

#[test]
fn random_bytes_never_panic() {
    let limits = ParseLimits::default();
    let mut rng = XorShift(0x2545_F491_4F6C_DD1D);
    for _ in 0..4_000 {
        let len = (rng.next() % 128) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let text = String::from_utf8_lossy(&bytes);
        let _ = parse_text(&text, &limits);
    }
}

#[test]
fn structured_random_input_never_panics_and_is_deterministic() {
    // 用有限的「结构化字母表」偏置，让随机输入真正打到段头 / 指令 / opcode 分支。
    const ALPHABET: &[u8] = b"<>/=$# \n\tabcABC019._-*?\"\\";
    let limits = ParseLimits::default();
    let mut rng = XorShift(0x9E37_79B9_7F4A_7C15);
    for _ in 0..4_000 {
        let len = (rng.next() % 96) as usize;
        let text: String = (0..len)
            .map(|_| char::from(ALPHABET[(rng.next() as usize) % ALPHABET.len()]))
            .collect();
        let first = parse_text(&text, &limits);
        let second = parse_text(&text, &limits);
        assert_eq!(format!("{first:?}"), format!("{second:?}"));
    }
}

#[test]
fn too_many_regions_hits_the_explicit_limit() {
    let mut text = String::new();
    for index in 0..8 {
        text.push_str(&format!("<region>sample=r{index}.wav\n"));
    }
    let limits = ParseLimits {
        max_regions: 4,
        ..ParseLimits::default()
    };
    let error = parse_text(&text, &limits).expect_err("must hit region cap");
    assert!(
        matches!(error, SfzError::TooManyRegions { limit: 4 }),
        "unexpected: {error:?}"
    );
}

#[test]
fn too_many_opcodes_hits_the_explicit_limit() {
    let text = "<global>\na1=1\na2=2\na3=3\na4=4\na5=5\n";
    let limits = ParseLimits {
        max_opcodes_per_header: 4,
        ..ParseLimits::default()
    };
    let error = parse_text(text, &limits).expect_err("must hit opcode cap");
    assert!(
        matches!(error, SfzError::TooManyOpcodes { limit: 4, .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn too_many_defines_hits_the_explicit_limit() {
    let text = "#define $A 1\n#define $B 2\n#define $C 3\n#define $D 4\n#define $E 5\n";
    let limits = ParseLimits {
        max_defines: 4,
        ..ParseLimits::default()
    };
    let error = parse_text(text, &limits).expect_err("must hit define cap");
    assert!(
        matches!(error, SfzError::TooManyDefines { limit: 4 }),
        "unexpected: {error:?}"
    );
}

#[test]
fn oversized_source_file_hits_the_explicit_limit() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    fs::write(root.join("big.sfz"), "<region>sample=a.wav\n".repeat(64)).expect("write");
    let limits = ParseLimits {
        max_source_bytes: 32,
        ..ParseLimits::default()
    };
    let resolver = IncludeResolver::new(root, limits).expect("base dir");
    let error = resolver.resolve("big.sfz").expect_err("must hit size cap");
    assert!(
        matches!(error, SfzError::SourceTooLarge { limit: 32, .. }),
        "unexpected: {error:?}"
    );
}

#[test]
fn parse_sources_on_empty_input_is_an_empty_instrument() {
    let limits = ParseLimits::default();
    let instrument = parse_text("", &limits).expect("empty input is valid");
    assert!(instrument.is_empty());
    assert_eq!(instrument.len(), 0);
    let sources = [yeban_sfz::SfzSource {
        path: "empty.sfz".to_string(),
        text: String::new(),
        first_line: 1,
    }];
    let instrument = parse_sources(&sources, &limits).expect("empty source is valid");
    assert!(instrument.is_empty());
}

#[test]
fn parse_text_reports_includes_it_did_not_resolve() {
    let instrument =
        parse_text("#include \"other.sfz\"\n", &ParseLimits::default()).expect("parses");
    assert!(
        instrument
            .warnings()
            .iter()
            .any(|warning| matches!(warning, yeban_sfz::Warning::IncludeIgnored { .. }))
    );
}

#[test]
fn unknown_headers_are_ignored_with_a_warning_not_treated_as_regions() {
    let instrument = parse_text(
        "<curve>\ncurve_index=1\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1);
    assert_eq!(instrument.regions()[0].sample, "a.wav");
    assert!(
        instrument
            .warnings()
            .iter()
            .any(|warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { name, .. } if name == "curve"))
    );
}

#[test]
fn round_robin_is_deterministic_across_independent_parses() {
    let text = "<group>key=36 seq_length=4\n\
                <region>seq_position=1 sample=k1.wav\n\
                <region>seq_position=2 sample=k2.wav\n\
                <region>seq_position=3 sample=k3.wav\n\
                <region>seq_position=4 sample=k4.wav";
    let limits = ParseLimits::default();
    let first = parse_text(text, &limits).expect("parses");
    let second = parse_text(text, &limits).expect("parses");
    let sequence = |instrument: &yeban_sfz::Instrument<'_>| {
        (0..8u64)
            .map(|occurrence| {
                instrument
                    .region_for_with(
                        yeban_sfz::RegionQuery::new(36, 100).with_occurrence(occurrence),
                    )
                    .map(|region| region.sample.to_string())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
    };
    let expected = vec![
        "k1.wav", "k2.wav", "k3.wav", "k4.wav", "k1.wav", "k2.wav", "k3.wav", "k4.wav",
    ];
    assert_eq!(sequence(&first), expected);
    assert_eq!(sequence(&second), expected);
}

#[test]
fn include_resolution_is_deterministic_across_runs() {
    let dir = TempDir::new("sfz");
    let root = dir.path();
    fs::create_dir_all(root.join("parts")).expect("dirs");
    for name in ["c.sfz", "a.sfz", "b.sfz"] {
        fs::write(
            root.join("parts").join(name),
            format!("<region>sample={name}.wav\n"),
        )
        .expect("write");
    }
    fs::write(root.join("main.sfz"), "#include \"parts/*.sfz\"\n").expect("write");

    let limits = ParseLimits::default();
    let run = || {
        let resolver = IncludeResolver::new(root, limits).expect("base dir");
        let sources = resolver.resolve("main.sfz").expect("resolves");
        let paths: Vec<String> = sources.iter().map(|source| source.path.clone()).collect();
        let instrument = parse_sources(&sources, &limits).expect("parses");
        let samples: Vec<String> = instrument
            .regions()
            .iter()
            .map(|region| region.sample.to_string())
            .collect();
        (paths, samples)
    };
    assert_eq!(run(), run());
}
