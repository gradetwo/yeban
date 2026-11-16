//! ⭐ `set_hdccN` / 废弃别名 `set_realccN`（ARIA 浮点 CC 初值）——第五份报告 §6 缺口。
//!
//! 规范逐字（<https://sfzformat.com/opcodes/set_hdccN/>）：
//!
//! > Like `set_ccN` but with floating point MIDI CCs. Sets a default initial value for
//! > MIDI CC number N using a floating point value, when the instrument is initially loaded.
//!
//! > `set_realccN` is a deprecated alias for `set_hdccN`.
//!
//! 判定量：`Instrument::cc_defaults()[cc]`（7 位整数，单位 CC 值 0..=127）。
//! 换算 **`round(v × 127)`**，优先级：`set_ccN`（SFZ v2 精确整数）> `set_hdccN`（ARIA 换算）。

use yeban_sfz::parser::{ParseLimits, Warning, parse_text};

/// `<control>` 段的浮点初值 ⇒ 7 位整数（0.5 → 64）。
#[test]
fn h1_set_hdcc_is_rounded_into_the_existing_seven_bit_domain() {
    let instrument = parse_text(
        "<control>set_hdcc10=0.5\n<region>sample=x.wav",
        &Default::default(),
    )
    .expect("解析");
    assert_eq!(
        instrument.cc_defaults().get(&10),
        Some(&64),
        "round(0.5 × 127) = 64"
    );
    // 端点逐一对：0 → 0、1 → 127。
    let ends = parse_text(
        "<control>set_hdcc7=0\nset_hdcc8=1\n<region>sample=x.wav",
        &Default::default(),
    )
    .expect("解析");
    assert_eq!(ends.cc_defaults().get(&7), Some(&0));
    assert_eq!(ends.cc_defaults().get(&8), Some(&127));
}

/// 废弃别名 `set_realccN` 与本体等价。
#[test]
fn h3_the_deprecated_set_realcc_alias_is_equivalent() {
    let hd = parse_text(
        "<control>set_hdcc16=0.5\n<region>sample=x.wav",
        &Default::default(),
    )
    .expect("解析");
    let real = parse_text(
        "<control>set_realcc16=0.5\n<region>sample=x.wav",
        &Default::default(),
    )
    .expect("解析");
    assert_eq!(hd.cc_defaults(), real.cc_defaults());
}

/// `set_ccN` 赢过 `set_hdccN`，且与书写顺序无关。
#[test]
fn h4_the_integer_set_cc_wins_over_the_float_extension() {
    for text in [
        "<control>set_cc10=100\nset_hdcc10=0.5\n<region>sample=x.wav",
        "<control>set_hdcc10=0.5\nset_cc10=100\n<region>sample=x.wav",
    ] {
        let instrument = parse_text(text, &ParseLimits::default()).expect("解析");
        assert_eq!(instrument.cc_defaults().get(&10), Some(&100), "{text}");
    }
    // 不同 CC 各自保留（合并，不是互相清空）。
    let mixed = parse_text(
        "<control>set_cc10=100\nset_hdcc11=0.5\n<region>sample=x.wav",
        &ParseLimits::default(),
    )
    .expect("解析");
    assert_eq!(mixed.cc_defaults().get(&10), Some(&100));
    assert_eq!(mixed.cc_defaults().get(&11), Some(&64));
}

/// 越界取值：丢弃该条 + 告警，⛔ 不中断整份乐器；下标越界：明确 Err。
#[test]
fn h2_out_of_range_values_warn_and_drop_but_index_overflow_is_an_error() {
    let instrument = parse_text(
        "<control>set_hdcc10=1.5\nset_hdcc11=-0.1\nset_hdcc12=NaN\n<region>sample=x.wav",
        &ParseLimits::default(),
    )
    .expect("越界取值不中断解析");
    assert!(instrument.cc_defaults().is_empty());
    let malformed = instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::MalformedSetCc { .. }))
        .count();
    assert_eq!(
        malformed,
        3,
        "三条越界声明各一条告警: {:?}",
        instrument.warnings()
    );

    let error = parse_text(
        "<control>set_hdcc65536=0.5\n<region>sample=x.wav",
        &ParseLimits::default(),
    )
    .expect_err("下标越界必须响亮");
    assert!(
        format!("{error:?}").contains("IntegerOutOfRange"),
        "{error:?}"
    );
}

/// 真正用到它的两份登记语料可加载，且初值正确。
#[test]
fn h6_corpus_instruments_load_with_the_declared_initial_values() {
    let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/samples");
    for (path, cc, expected) in [
        (
            "salamander-grand/Salamander Grand Piano V3.sfz",
            10u16,
            64u8,
        ),
        ("ixox-flute/Ixox Flute.sfz", 10, 64),
    ] {
        let full = format!("{root}/{path}");
        let text = std::fs::read_to_string(&full).expect("语料文件在仓库里");
        let instrument = parse_text(&text, &ParseLimits::default()).expect("语料必须可加载");
        assert_eq!(
            instrument.cc_defaults().get(&cc),
            Some(&expected),
            "{path} 的 set_hdcc{cc}"
        );
    }
}
