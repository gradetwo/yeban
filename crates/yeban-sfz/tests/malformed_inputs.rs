//! 畸形输入 / 显式上限 / 确定性判据。
//!
//! 这是 `MUST-GATE-011`（cargo-fuzz 千万次变异零崩溃）在**本机可跑**的那一层：
//! 一组固定畸形样本 + 确定性伪随机字节流（含结构化字母表偏置），断言解析器
//! **只返回 `Result`，永不 panic**。真正的 cargo-fuzz 目标在 `fuzz/`，本机不跑（见 notes）。
//!
//! 判据纪律：这些测试都曾经在「故意改坏实现」时变红，见
//! `docs/ledger/sfz-core-notes.md` 的「注入记录」表。

use std::fs;

use yeban_sfz::{
    IncludeResolver, PITCH_BEND_CENTER, ParseLimits, SfzError, parse_sources, parse_text,
};

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
    "<region>sample=a.wav offset=",
    "<region>sample=a.wav offset=-1",
    "<region>sample=a.wav offset=4294967296",
    "<region>sample=a.wav offset=99999999999999999999",
    "<region>sample=a.wav end=",
    "<region>sample=a.wav end=-2",
    "<region>sample=a.wav end=4294967296",
    "<region>sample=a.wav end=abc",
    "<region>sample=a.wav offset=200 end=100",
    "<region>sample=a.wav direction=",
    "<region>sample=a.wav direction=sideways",
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
    // `<curve>` 段（1 个合法块 + 各种畸形）：任何字节都只允许 Ok / Err，不允许 panic。
    "<curve>curve_index=7",
    "<curve>curve_index=7\nv000=0\nv127=1\n<region>sample=a.wav",
    "<curve>curve_index=7\nv000=nan",
    "<curve>curve_index=7\nv000=1e40",
    "<curve>curve_index=7\nv128=0",
    "<curve>curve_index=7\nv=0\nv00=0\nv0000=0\nv999=0",
    "<curve>curve_index=0\nv000=0",
    "<curve>curve_index=6\nv000=-1",
    "<curve>curve_index=-1\nv000=0",
    "<curve>curve_index=255\nv000=0",
    "<curve>curve_index=99999999999999999999999\nv000=0",
    "<curve>curve_index=",
    "<curve>curve_index",
    "<curve>v000=0",
    "<curve>curve_index=7\nv000=\n<curve>curve_index=7",
    "<CURVE>curve_index=7\nv000=0",
    "<curve>>curve_index=7",
    "<curve>curve_index=7\n\n// 注释\nv127=1",
    // `<effect>` 段（含 19 个登记语料文件用的 ARIA MDA 形状 + 各种畸形）：
    // 任何字节都只允许 Ok / Err，不允许 panic。
    "<effect>bus=aux1 type=com.mda.Limiter param_offset=400 dsp_order=2 effect1=50",
    "<effect>",
    "<effect>\n",
    "<effect>bus=",
    "<effect>bus=aux0",
    "<effect>bus=aux9",
    "<effect>bus=fx5",
    "<effect>bus=main\nbus=aux2\nbus=",
    "<effect>dsp_order=0",
    "<effect>dsp_order=14",
    "<effect>dsp_order=15",
    "<effect>dsp_order=-1",
    "<effect>dsp_order=",
    "<effect>dsp_order=99999999999999999999",
    "<effect>param_offset=0",
    "<effect>param_offset=-1",
    "<effect>param_offset=4294967296",
    "<effect>param_offset=",
    "<effect>effect1=0\neffect4=100",
    "<effect>effect1=NaN",
    "<effect>effect2=inf",
    "<effect>effect3=-100",
    "<effect>effect0=1\neffect5=1\neffect=1",
    "<effect>type=",
    "<effect>type=com.mda.Limiter\nparam_offset=400",
    "<effect>fx1=1\n<midi>midi_cc1=64\n<sample>sample=b.wav\n<region>sample=a.wav",
    "<EFFECT>bus=aux1",
    "<effect>bus=aux1\n\n// 注释\ntype=com.mda.Limiter",
    "<master>key=36",
    "<midi>",
    "<midi>\ncc1=64\ncurve_index=7\n<region>sample=a.wav",
    "<midi>cc1=\ncc2=not-a-number\ncc3=99999999999999999999",
    "<midi>cc1=64\n<midi>cc1=1\n<midi>\n<region>sample=a.wav",
    "<MIDI>cc1=64",
    "<midi >cc1=64",
    "<midi>cc1=\"quoted value\"",
    "<midi>$UNDEFINED=1",
    "<midi>cc1=64 // comment\ncc2=1",
    "<sample>",
    "sample=a.wav",
    "=a.wav",
    "key=36",
    "<region>sample=a.wav key=36 key=48",
    // `off_time` 已建模（见 `off_time_is_modeled_and_range_checked_explicitly`）：
    // 这里只要求「任何字节都只允许 Ok / Err，不允许 panic」。
    "<region>sample=a.wav off_mode=time off_time=0.05",
    "<region>sample=a.wav off_mode=time off_time=0",
    "<region>sample=a.wav off_time=",
    "<region>sample=a.wav off_time=abc",
    "<region>sample=a.wav off_time=NaN",
    "<region>sample=a.wav off_time=inf",
    "<region>sample=a.wav off_time=-inf",
    "<region>sample=a.wav off_time=-0.0",
    "<region>sample=a.wav off_time=-1",
    "<region>sample=a.wav off_time=1e999",
    "<region>sample=a.wav off_time=99999999999999999999",
    "<region>sample=a.wav off_time=0.05 off_time=-0.05",
    "<group>off_time=0.25\n<region>sample=a.wav",
    // `amp_veltrack` / `amp_velcurve_N` 已建模（见
    // `velocity_response_is_modeled_and_range_checked_explicitly`）：
    // 这里只要求「任何字节都只允许 Ok / Err，不允许 panic」。
    "<region>sample=a.wav amp_veltrack=",
    "<region>sample=a.wav amp_veltrack=abc",
    "<region>sample=a.wav amp_veltrack=NaN",
    "<region>sample=a.wav amp_veltrack=inf",
    "<region>sample=a.wav amp_veltrack=-inf",
    "<region>sample=a.wav amp_veltrack=-100",
    "<region>sample=a.wav amp_veltrack=100",
    "<region>sample=a.wav amp_veltrack=-100.0001",
    "<region>sample=a.wav amp_veltrack=101",
    "<region>sample=a.wav amp_veltrack=1e999",
    "<region>sample=a.wav amp_veltrack=99999999999999999999",
    "<region>sample=a.wav amp_velcurve_=",
    "<region>sample=a.wav amp_velcurve_foo=0.5",
    "<region>sample=a.wav amp_velcurve_1x=0.5",
    "<region>sample=a.wav amp_velcurve_1=",
    "<region>sample=a.wav amp_velcurve_1=abc",
    "<region>sample=a.wav amp_velcurve_1=NaN",
    "<region>sample=a.wav amp_velcurve_1=inf",
    "<region>sample=a.wav amp_velcurve_1=-0.0",
    "<region>sample=a.wav amp_velcurve_1=-1",
    "<region>sample=a.wav amp_velcurve_1=1.0000001",
    "<region>sample=a.wav amp_velcurve_1=2",
    "<region>sample=a.wav amp_velcurve_1=1e999",
    "<region>sample=a.wav amp_velcurve_0=0 amp_velcurve_127=1",
    "<region>sample=a.wav amp_velcurve_127=0.5",
    "<region>sample=a.wav amp_velcurve_128=0.5",
    "<region>sample=a.wav amp_velcurve_255=0.5",
    "<region>sample=a.wav amp_velcurve_256=0.5",
    "<region>sample=a.wav amp_velcurve_99999999999999999999=0.5",
    "<region>sample=a.wav amp_velcurve_000000000000000000001=0.5",
    "<region>sample=a.wav amp_velcurve_+1=0.5",
    "<global>amp_velcurve_64=0.1\n<master>amp_velcurve_64=0.2\n<group>amp_velcurve_64=0.3\n<region>sample=a.wav amp_velcurve_64=0.4",
    "<region>sample=a.wav amp_velcurve_1=0.2 amp_velcurve_3=0.3",
    "<region>sample=a.wav amp_veltrack=0 amp_velcurve_1=0.5",
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
    // `<curve>` / `<effect>` / `<midi>` 已建模（见各自的 `*_is_no_longer_an_ignored_header`
    // 测试），这里用仍未建模的 `<sample>` 守同一条红线：
    // 未实现的段头必须产生告警并丢掉段内 opcode，绝不当作 region。
    let instrument = parse_text(
        "<sample>\nsample=b.wav\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1, "only the <region> becomes a region");
    assert_eq!(instrument.regions()[0].sample, "a.wav");
    assert!(
        instrument.warnings().iter().any(
            |warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { name, .. } if name == "sample")
        ),
        "sample must be reported as an ignored header"
    );
}

#[test]
fn midi_header_is_modeled_and_is_no_longer_an_ignored_header() {
    // 登记语料（`git ls-files` 里的 1 398 个 `.sfz`）**没有**任何 `<midi>` 段，
    // 所以这里的形状取自规范页与 `EffectBus::Midi` 已经记下的等价关系，而不是语料。
    let instrument = parse_text(
        "<midi>cc1=64\ncurve_index=7\n\n//Curves\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1, "the <region> after the <midi> is kept");
    assert_eq!(instrument.regions()[0].sample, "a.wav");
    assert!(
        !instrument
            .warnings()
            .iter()
            .any(|warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { name, .. } if name == "midi")),
        "<midi> is modeled now: it must not be reported as an ignored header"
    );
    let sections = instrument.midi_sections();
    assert_eq!(sections.len(), 1);
    assert_eq!(sections[0].opcode("cc1"), Some("64"));
    assert_eq!(sections[0].opcode("curve_index"), Some("7"));
    assert!(instrument.midi_preprocessor_declared());
}

#[test]
fn effect_header_is_modeled_and_is_no_longer_an_ignored_header() {
    // 19 个登记语料文件里 `<effect>` 的形状（`assets/samples/karoryfer-big-rusty-drums/…`）：
    // `param_offset` + ARIA 的 MDA `type`。
    let instrument = parse_text(
        "<effect>\nparam_offset=400\ntype=com.mda.Limiter\n\n//Curves\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1, "the <region> after the effect is kept");
    assert_eq!(instrument.regions()[0].sample, "a.wav");
    assert!(
        !instrument
            .warnings()
            .iter()
            .any(|warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { name, .. } if name == "effect")),
        "<effect> is modeled now: it must not be reported as an ignored header"
    );
    let effects = instrument.effects();
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].type_name(), Some("com.mda.Limiter"));
    assert_eq!(effects[0].param_offset(), Some(400));
    assert_eq!(effects[0].bus(), yeban_sfz::EffectBus::Main);
}

#[test]
fn effect_header_does_not_clear_the_inheritance_chain() {
    // 定义段语义：`<effect>` 既不写进继承链、也不清空它。
    // 反例（改动前不可能发生，改动后也必须不发生的回归）：若 `<effect>` 清空作用域，
    // 下面 region 的 `key` / `volume` 都会丢失。
    let instrument = parse_text(
        "<group>key=36 volume=-3\n<effect>bus=aux1 type=comp\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1);
    let region = &instrument.regions()[0];
    assert_eq!(region.lokey, 36, "group key survives the <effect> section");
    assert_eq!(region.hikey, 36);
    assert_eq!(region.volume, -3.0, "group volume survives");
    // 同一文件里多个 `<effect>`：全部保留（同一条总线上可以串多级效果，不去重）。
    let instrument = parse_text(
        "<effect>bus=aux1\n<effect>bus=aux1 dsp_order=1\n<effect>bus=fx2 type=comp\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    let buses: Vec<yeban_sfz::EffectBus> = instrument
        .effects()
        .iter()
        .map(yeban_sfz::Effect::bus)
        .collect();
    assert_eq!(
        buses,
        vec![
            yeban_sfz::EffectBus::Aux(1),
            yeban_sfz::EffectBus::Aux(1),
            yeban_sfz::EffectBus::Fx(2)
        ]
    );
}

#[test]
fn an_effect_section_without_normative_opcodes_produces_nothing_and_no_error() {
    // 空段 / 只写非规范 opcode 的段：没有数据可丢 ⇒ 不产生条目也不报错（与 `<curve>` 同口径）。
    // 注意 `param_offset` 是 ARIA 为 `<effect>` 文档化的 opcode（`/opcodes/param_offset/`），
    // 所以它**不算**非规范；这里用的 `fx1` 才是 Rapture 的厂商私有名字。
    for source in [
        "<effect>",
        "<effect>\n",
        "<effect>foo=1\n",
        "<effect>fx1=1\neffect0=1\neffect5=1\n",
    ] {
        let instrument = parse_text(source, &ParseLimits::default()).expect("parses");
        assert!(
            instrument.effects().is_empty(),
            "no normative opcode in {source:?} ⇒ no effect entry, got {:?}",
            instrument.effects()
        );
        assert!(
            !instrument
                .warnings()
                .iter()
                .any(|warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { .. })),
            "a modeled header must not warn as ignored: {source:?}"
        );
    }
}

#[test]
fn an_unknown_bus_value_falls_back_to_main_without_an_error() {
    // 规范原文（<https://sfzformat.com/opcodes/bus/>）：
    // "If not set, or any other value is set, this goes to the main output."
    // ⇒ 未知 `bus` 取值**不是** Err，而是归约到主输出；`is_option` 负责机械区分。
    for value in ["not-a-bus", "aux9", "fx5", ""] {
        let source = format!("<effect>bus={value}");
        let instrument = parse_text(&source, &ParseLimits::default()).expect("parses");
        assert_eq!(instrument.effects().len(), 1, "bus= is a normative opcode");
        assert_eq!(
            instrument.effects()[0].bus(),
            yeban_sfz::EffectBus::Main,
            "bus={value:?} resolves to the main output"
        );
        assert!(
            !yeban_sfz::EffectBus::is_option(value),
            "bus={value:?} is not a listed option"
        );
    }
    // `main` 是表里的名字，大小写不敏感（与其它 option opcode 同口径）。
    for value in ["main", "MAIN", " main "] {
        assert!(yeban_sfz::EffectBus::is_option(value), "bus={value:?}");
        assert_eq!(
            yeban_sfz::EffectBus::from_value(value),
            yeban_sfz::EffectBus::Main
        );
    }
}

#[test]
fn too_many_effects_hits_the_explicit_limit() {
    let limits = ParseLimits {
        max_effects: 1,
        ..ParseLimits::default()
    };
    let error = parse_text("<effect>bus=aux1\n<effect>bus=aux2\n", &limits)
        .expect_err("second effect exceeds max_effects");
    assert!(
        matches!(error, SfzError::TooManyEffects { limit: 1 }),
        "unexpected error: {error:?}"
    );
}

#[test]
fn effect_dsp_order_and_param_offset_are_range_checked_explicitly() {
    let limits = ParseLimits::default();
    for good in ["0", "14"] {
        let source = format!("<effect>dsp_order={good}");
        let instrument = parse_text(&source, &limits).expect("in range");
        assert_eq!(
            instrument.effects()[0].dsp_order(),
            Some(good.parse::<u8>().expect("digit")),
            "dsp_order={good}"
        );
    }
    for bad in ["15", "-1", "abc", "99999999999999999999"] {
        let source = format!("<effect>dsp_order={bad}");
        assert!(
            parse_text(&source, &limits).is_err(),
            "dsp_order={bad} must be an explicit Err, not a silent clamp/default"
        );
    }
    for bad in ["-1", "abc", "4294967296"] {
        let source = format!("<effect>param_offset={bad}");
        assert!(
            parse_text(&source, &limits).is_err(),
            "param_offset={bad} must be an explicit Err"
        );
    }
    for bad in ["NaN", "inf", "-inf", ""] {
        let source = format!("<effect>effect1={bad}");
        assert!(
            parse_text(&source, &limits).is_err(),
            "effect1={bad} must be an explicit Err"
        );
    }
}

#[test]
fn curve_header_is_modeled_and_is_no_longer_an_ignored_header() {
    let instrument = parse_text(
        "<curve>curve_index=7\nv000=0\nv095=1\nv127=1\n<region>sample=a.wav\n",
        &ParseLimits::default(),
    )
    .expect("parses");
    assert_eq!(instrument.len(), 1);
    assert_eq!(instrument.regions()[0].sample, "a.wav");
    assert!(
        !instrument
            .warnings()
            .iter()
            .any(|warning| matches!(warning, yeban_sfz::Warning::IgnoredHeader { name, .. } if name == "curve")),
        "<curve> is modeled now: it must not be reported as an ignored header"
    );
    let curves = instrument.curves();
    assert_eq!(curves.len(), 1);
    assert_eq!(curves[0].index(), 7);
    assert_eq!(instrument.curve_value_at(7, 95.0), Some(1.0));
    assert_eq!(instrument.curve(7).map(yeban_sfz::Curve::index), Some(7));
    // 内建曲线仍然可取（`curve_index` 0..=6 不可覆写，所以文件里定义的编号必然 ≥ 7）。
    assert_eq!(instrument.curve_value_at(1, 0.0), Some(-1.0));
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

#[test]
fn off_time_is_modeled_and_range_checked_explicitly() {
    let limits = ParseLimits::default();

    // 缺省：`None` + 规范缺省 0.006 秒；显式 0 必须存活为 0（二者不是同一个请求）。
    let default = parse_text("<region>sample=a.wav", &limits).expect("parses");
    assert_eq!(default.regions()[0].off_time, None);
    assert_eq!(default.regions()[0].effective_off_time(), 0.006);
    let zero = parse_text("<region>sample=a.wav off_time=0", &limits).expect("parses");
    assert_eq!(zero.regions()[0].off_time, Some(0.0));
    assert_eq!(zero.regions()[0].effective_off_time(), 0.0);

    // 语料里出现的 6 个不同取值全部读得到（扫描口径见本票报告）
    for text in ["0.05", "0.5", "0.25", "0.2", "0.4", "0.3"] {
        let source = format!("<group>off_mode=time off_time={text}\n<region>sample=a.wav");
        let instrument = parse_text(&source, &limits).expect("parses");
        let expected: f32 = text.parse().expect("literal parses as f32");
        assert_eq!(
            instrument.regions()[0].off_time,
            Some(expected),
            "off_time={text}"
        );
    }

    // 负数是明确 Err；非数字 / 非有限值是明确 Err；两者都**不**静默回退到缺省。
    let negative = parse_text("<region>sample=a.wav off_time=-0.05", &limits);
    assert!(
        matches!(negative, Err(SfzError::InvalidDuration { .. })),
        "a negative off_time must be an explicit Err, got {negative:?}"
    );
    for bad in ["", "abc", "NaN", "inf", "-inf", "1e999"] {
        let source = format!("<region>sample=a.wav off_time={bad}");
        let outcome = parse_text(&source, &limits);
        assert!(
            matches!(
                outcome,
                Err(SfzError::InvalidFloat { .. } | SfzError::NonFiniteFloat { .. })
            ),
            "off_time={bad:?} must be an explicit Err, got {outcome:?}"
        );
    }
}

#[test]
fn velocity_response_is_modeled_and_range_checked_explicitly() {
    let limits = ParseLimits::default();

    // 缺省：`amp_veltrack` 是规范缺省 100，且没有点表 ⇒ 力度响应用标准曲线。
    let default = parse_text("<region>sample=a.wav", &limits).expect("parses");
    let region = &default.regions()[0];
    assert_eq!(region.amp_veltrack, 100.0);
    assert!(region.velocity_curve.is_none());
    assert_eq!(region.velocity_gain(127), 1.0);
    assert!((region.velocity_gain(64) - (64.0f32 / 127.0).powi(2)).abs() <= 1.0e-6);

    // 语料里出现的 5 个不同 `amp_veltrack` 取值全部读得到（扫描口径见本票报告）。
    for text in ["0", "100", "50", "40", "35"] {
        let source = format!("<region>sample=a.wav amp_veltrack={text}");
        let instrument = parse_text(&source, &limits).expect("parses");
        let expected: f32 = text.parse().expect("literal parses as f32");
        assert_eq!(instrument.regions()[0].amp_veltrack, expected, "{text}");
    }

    // `amp_veltrack` 越界是明确 Err；边界（含）合法；非数字 / 非有限值是明确 Err。
    for bad in ["101", "-100.5", "99999999999999999999"] {
        let source = format!("<region>sample=a.wav amp_veltrack={bad}");
        let outcome = parse_text(&source, &limits);
        assert!(
            matches!(outcome, Err(SfzError::FloatOutOfRange { .. })),
            "amp_veltrack={bad:?} must be an explicit FloatOutOfRange, got {outcome:?}"
        );
    }
    for bad in ["", "abc", "NaN", "inf", "-inf", "1e999"] {
        let source = format!("<region>sample=a.wav amp_veltrack={bad}");
        let outcome = parse_text(&source, &limits);
        assert!(
            matches!(
                outcome,
                Err(SfzError::InvalidFloat { .. } | SfzError::NonFiniteFloat { .. })
            ),
            "amp_veltrack={bad:?} must be an explicit Err, got {outcome:?}"
        );
    }
    for good in ["-100", "100"] {
        let source = format!("<region>sample=a.wav amp_veltrack={good}");
        parse_text(&source, &limits).expect("range endpoints are inside");
    }

    // 下标 `N` 的取值域是规范正文的 0..=127：越界与解析溢出都必须是明确 Err
    // （不静默丢弃该点 —— 那会让力度曲线无声地变错）。
    let index = parse_text("<region>sample=a.wav amp_velcurve_128=0.5", &limits);
    assert!(
        matches!(
            index,
            Err(SfzError::VelocityCurveIndexOutOfRange { ref index, .. }) if index == "128"
        ),
        "amp_velcurve_128 must be an explicit Err, got {index:?}"
    );
    let overflow = parse_text(
        "<region>sample=a.wav amp_velcurve_99999999999999999999=0.5",
        &limits,
    );
    assert!(
        matches!(overflow, Err(SfzError::VelocityCurveIndexOutOfRange { .. })),
        "an overflowing index must be an explicit Err, got {overflow:?}"
    );

    // 取值域是规范表格的 0..=1：越界是明确 Err，端点（含）合法。
    let amplitude = parse_text("<region>sample=a.wav amp_velcurve_1=1.5", &limits);
    assert!(
        matches!(
            amplitude,
            Err(SfzError::FloatOutOfRange {
                min: 0.0,
                max: 1.0,
                ..
            })
        ),
        "amp_velcurve_1=1.5 must be an explicit FloatOutOfRange, got {amplitude:?}"
    );
    parse_text(
        "<region>sample=a.wav amp_velcurve_1=0 amp_velcurve_2=1",
        &limits,
    )
    .expect("0 and 1 are inside the range");

    // 规范原文算例（<https://sfzformat.com/opcodes/amp_velcurve_N/>）：
    // `amp_velcurve_1=0.2 amp_velcurve_3=0.3` ⇒ `amp_velcurve_2` 是 0.25。
    let worked = parse_text(
        "<region>sample=a.wav amp_velcurve_1=0.2 amp_velcurve_3=0.3",
        &limits,
    )
    .expect("parses");
    assert_eq!(worked.regions()[0].velocity_gain(2), 0.25);
    assert_eq!(worked.regions()[0].velocity_gain(0), 0.0);
    assert_eq!(worked.regions()[0].velocity_gain(127), 1.0);

    // 显式点表压过 `amp_veltrack`（本 crate 的工程裁决）；只有当点表缺席时
    // `amp_veltrack=0` 才给出恒等。
    let combined = parse_text(
        "<region>sample=a.wav amp_veltrack=0 amp_velcurve_1=0.5",
        &limits,
    )
    .expect("parses");
    assert_eq!(combined.regions()[0].velocity_gain(1), 0.5);
    let tracked_only = parse_text("<region>sample=a.wav amp_veltrack=0", &limits).expect("parses");
    assert_eq!(tracked_only.regions()[0].velocity_gain(1), 1.0);

    // 四级作用域链：region → group → master → global，内层胜。
    let scoped = parse_text(
        "<global>amp_velcurve_64=0.1\n\
         <master>amp_velcurve_64=0.2 amp_velcurve_32=0.7\n\
         <group>amp_velcurve_64=0.3\n\
         <region>sample=a.wav amp_velcurve_64=0.4",
        &limits,
    )
    .expect("parses");
    assert_eq!(scoped.regions()[0].velocity_gain(64), 0.4);
    assert_eq!(scoped.regions()[0].velocity_gain(32), 0.7);
}

#[test]
fn bend_range_is_modeled_and_range_checked_explicitly() {
    let limits = ParseLimits::default();

    // 缺省：规范表格（<https://sfzformat.com/opcodes/bend_up/>、
    // <https://sfzformat.com/opcodes/bend_down/>）的 Default 列是 200 / -200，
    // 范围都是 -9600 to 9600。缺省**不是** 0：缺省轮子推到底要弯两个半音。
    let default = parse_text("<region>sample=a.wav", &limits).expect("parses");
    let region = &default.regions()[0];
    assert_eq!(region.bend_up, 200);
    assert_eq!(region.bend_down, -200);
    assert_eq!(region.bend_cents(PITCH_BEND_CENTER), 0);
    assert_eq!(region.bend_cents(127), 200);
    assert_eq!(region.bend_cents(0), -200);

    // 语料里两个 opcode 的 13 个不同取值全部读得到（扫描口径见本票报告）：
    // 含 `bend_up=0` / `bend_down=0`（显式 0 ≠ 缺省）与 `bend_down=1200`（正值合法）。
    for (opcode, literals) in [
        ("bend_up", &["0", "300", "500", "1200", "2400"][..]),
        (
            "bend_down",
            &[
                "-3600", "-2400", "-1200", "-700", "-500", "-400", "0", "1200",
            ][..],
        ),
    ] {
        for literal in literals {
            let source = format!("<region>sample=a.wav {opcode}={literal}");
            let instrument = parse_text(&source, &limits).expect("parses");
            let expected: i32 = literal.parse().expect("literal parses as i32");
            let parsed = if opcode == "bend_up" {
                instrument.regions()[0].bend_up
            } else {
                instrument.regions()[0].bend_down
            };
            assert_eq!(parsed, expected, "{opcode}={literal}");
        }
    }

    // 越界是明确 Err（不静默钳位）；端点（含）合法。
    for bad in ["9601", "-9601"] {
        for opcode in ["bend_up", "bend_down"] {
            let source = format!("<region>sample=a.wav {opcode}={bad}");
            let outcome = parse_text(&source, &limits);
            assert!(
                matches!(
                    outcome,
                    Err(SfzError::IntegerOutOfRange {
                        min: -9600,
                        max: 9600,
                        ..
                    })
                ),
                "{opcode}={bad} must be an explicit IntegerOutOfRange, got {outcome:?}"
            );
        }
    }
    parse_text("<region>sample=a.wav bend_up=9600 bend_down=-9600", &limits)
        .expect("the range endpoints are inside");

    // 非整数 / 空值 / 超长数字都是明确 Err，不是 panic，也不是静默缺省。
    for bad in ["", "abc", "12.5", "1e3", "99999999999999999999"] {
        for opcode in ["bend_up", "bend_down"] {
            let source = format!("<region>sample=a.wav {opcode}={bad}");
            let outcome = parse_text(&source, &limits);
            assert!(
                matches!(
                    outcome,
                    Err(SfzError::InvalidInteger { .. } | SfzError::IntegerOutOfRange { .. })
                ),
                "{opcode}={bad:?} must be an explicit Err, got {outcome:?}"
            );
        }
    }

    // 中位轮值下音高比与静态音高比逐位相同（`2^0 = 1` 是乘法单位元）：
    // 这条把「新增字段不改变既有消费方电平」钉成位级判据。
    let bent = parse_text(
        "<region>sample=a.wav pitch_keycenter=60 bend_up=1200 bend_down=-1200",
        &limits,
    )
    .expect("parses");
    let plain = parse_text("<region>sample=a.wav pitch_keycenter=60", &limits).expect("parses");
    for note in [0u8, 36, 60, 127] {
        assert_eq!(
            bent.regions()[0]
                .bend_ratio(note, PITCH_BEND_CENTER)
                .to_bits(),
            plain.regions()[0].pitch_ratio(note).to_bits(),
            "note {note}"
        );
    }
}
