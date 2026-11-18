//! ⭐ **调制深度钳位判据**（Wang 2026-11-18 裁决）。
//!
//! # 裁决与本判据在防什么
//!
//! 登记语料里 4 款 Karoryfer 乐器在 `<global>` 写 `pan_oncc10=200`
//! （`pan` 的 Modulation 表域是 `-100..=100`），旧口径下调制量越界是明确
//! `Err` ⇒ **整份乐器不可加载**。裁决：**只有调制量**（modulation depth）
//! 越界时钳位到该参数自身值域的边界，并经装载报告的既有告警通道
//! （[`Warning::ClampedModulationDepth`]）响亮记录；静态参数值越界维持
//! 严格报错、非数值 / 非有限维持现行错误、下标类（曲线下标 / CC 号）
//! 不是量、维持严格报错。
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | 1 | `pan_oncc10=200` 可装载、深度落 100、告警载荷逐字可查 | `CcMod::delta` ＋ `Warning::ClampedModulationDepth` |
//! | 2 | `pan_oncc10=-200` 钳到 −100 | 同上 |
//! | 3 | 域内调制量（含两端点）不产生钳位告警 | `warnings()` 为空 |
//! | 4 | 静态与畸形**没有**被一并放宽 | `amp_veltrack=101` 仍 `Err`；`pan_oncc10=abc` / `=inf` 仍 `Err`；曲线下标 / CC 号越界仍 `Err`；静态 `pan=200` 的口径已由 Wang 2026-11-18 后续裁决改钉（装载期钳位 ＋ `ClampedStaticValue` 告警，完整判据在 `tests/static_pan_clamp.rs`） |
//! | 5 | 4 款真实 Karoryfer 入口装载成功且 region 数 > 0、`pan_oncc10` 钳位告警恰 1 条（跨 region 去重后） | include 展开 ＋ 解析归约（**装载级**；样本本体不在仓内，样本解码不在本 crate，不在本判据） |

use std::path::{Path, PathBuf};

use yeban_sfz::curve::Curve;
use yeban_sfz::modulation::{
    CcMod, ModTarget, bake_cc_mods, position_value_with_baked, width_value_with_baked,
};
use yeban_sfz::{IncludeResolver, ParseLimits, SfzError, Warning, parse_sources, parse_text};

/// 一条输入里全部钳位告警（顺序即装载报告顺序）。
fn clamp_notes(text: &str) -> Vec<Warning> {
    let instrument = parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"));
    instrument
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::ClampedModulationDepth { .. }))
        .cloned()
        .collect()
}

/// 取唯一的 `pan` 调制项。
fn the_pan_mod(text: &str) -> CcMod {
    let instrument = parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"));
    let mods: Vec<&CcMod> = instrument.regions()[0]
        .cc_modulations()
        .iter()
        .filter(|item| item.target == ModTarget::Pan)
        .collect();
    assert_eq!(mods.len(), 1, "`{text}` 必须恰一条 pan 调制项");
    *mods[0]
}

#[test]
fn pan_oncc_depth_above_the_range_is_clamped_to_100_with_a_warning() {
    let text = "<region> sample=a.wav pan_oncc10=200";
    let pan_mod = the_pan_mod(text);
    assert_eq!(pan_mod.cc, 10);
    assert_eq!(pan_mod.delta, 100.0, "深度必须落到值域上界 100");
    let notes = clamp_notes(text);
    assert_eq!(
        notes,
        vec![Warning::ClampedModulationDepth {
            line: 1,
            opcode: "pan_oncc10".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }],
        "告警必须在装载报告里逐字可查（行号 / opcode / 原值 / 钳位值）"
    );
    assert_eq!(
        notes[0].to_string(),
        "line 1: `pan_oncc10` modulation depth 200 out of range, clamped to 100"
    );
}

#[test]
fn pan_oncc_depth_below_the_range_is_clamped_to_minus_100() {
    let text = "<region> sample=a.wav pan_oncc10=-200";
    assert_eq!(the_pan_mod(text).delta, -100.0);
    assert_eq!(
        clamp_notes(text),
        vec![Warning::ClampedModulationDepth {
            line: 1,
            opcode: "pan_oncc10".to_string(),
            value: "-200".to_string(),
            clamped: "-100".to_string(),
        }]
    );
}

#[test]
fn in_range_modulation_depths_produce_no_clamp_warning() {
    // 两端点与域内值都**不**告警（钳位只发生在真的越界时）。
    for (text, want) in [
        ("<region> sample=a.wav pan_oncc10=100", 100.0),
        ("<region> sample=a.wav pan_oncc10=-100", -100.0),
        ("<region> sample=a.wav pan_oncc10=0", 0.0),
        ("<region> sample=a.wav pan_oncc10=42.5", 42.5),
    ] {
        assert_eq!(the_pan_mod(text).delta, want, "`{text}` 的深度必须原样");
        let instrument = parse_text(text, &ParseLimits::default()).expect("必须可装载");
        assert!(
            instrument.warnings().is_empty(),
            "`{text}` 在域内不得产生任何告警，实得 {:?}",
            instrument.warnings()
        );
    }
}

#[test]
fn statics_and_malformed_values_are_not_relaxed() {
    // 静态参数越界维持严格报错 —— 即使同一份文件里有一条已被钳位的调制量。
    let outcome = parse_text(
        "<region> sample=a.wav pan_oncc10=200 amp_veltrack=101",
        &ParseLimits::default(),
    );
    assert!(
        matches!(
            outcome,
            Err(SfzError::FloatOutOfRange { ref opcode, .. }) if opcode == "amp_veltrack"
        ),
        "静态 `amp_veltrack=101` 必须仍是 FloatOutOfRange，实得 {outcome:?}"
    );
    // ⭐ **改钉（Wang 2026-11-18 后续裁决）**：本块原钉「静态 `pan=200`
    // 原样带出、无告警、读数 200」（钳位不泄漏到静态路径）；后续裁决把
    // 声像族静态值也纳入钳位 ＋ 告警 ⇒ 现钉新口径：`pan=200` 可装载、
    // 读数钳到 100、恰一条 `ClampedStaticValue` 告警，且**没有**
    // `ClampedModulationDepth`（静态与调制两个变体不混）。完整判据
    // （position / width / 去重 / 真实文件回归）在
    // `tests/static_pan_clamp.rs`。
    let instrument = parse_text("<region> sample=a.wav pan=200", &ParseLimits::default())
        .expect("静态 pan 越界必须可装载（钳位、不拒载）");
    assert_eq!(instrument.regions()[0].pan, 100.0);
    assert_eq!(
        instrument.warnings(),
        &[Warning::ClampedStaticValue {
            line: 1,
            opcode: "pan".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }]
    );
    // 调制量的畸形取值维持现行错误（认不出 / 非有限都不是「越界」）。
    assert!(matches!(
        parse_text(
            "<region> sample=a.wav pan_oncc10=abc",
            &ParseLimits::default()
        ),
        Err(SfzError::InvalidFloat { .. })
    ));
    assert!(matches!(
        parse_text(
            "<region> sample=a.wav pan_oncc10=inf",
            &ParseLimits::default()
        ),
        Err(SfzError::NonFiniteFloat { .. })
    ));
    // 下标类不是量：曲线下标越界、CC 号越界维持严格报错。
    assert!(matches!(
        parse_text(
            "<region> sample=a.wav pan_oncc10=50 pan_curvecc10=300",
            &ParseLimits::default()
        ),
        Err(SfzError::IntegerOutOfRange { .. })
    ));
    assert!(matches!(
        parse_text(
            "<region> sample=a.wav pan_oncc300=50",
            &ParseLimits::default()
        ),
        Err(SfzError::IntegerOutOfRange { .. })
    ));
}

/// 语料根（相对 crate 根：`crates/yeban-sfz/../../assets/samples`）。
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples")
}

#[test]
fn the_four_karoryfer_instruments_load_with_regions_and_clamp_warnings() {
    // 修复前 4 款全部在第一条 region 归约时以
    // 「`pan_oncc10` value 200 out of range -100..=100」整份失败
    // （全库装载审计 `load-audit-baseline.md`）。修复后：装载成功、
    // region 数 > 0、点名 `pan_oncc10` 的钳位告警恰 1 条（跨 region 去重）。
    // ⚠ 深度只到**装载级**（include 展开 ＋ 解析 ＋ 归约）：样本本体
    // 不在仓内，样本解码不在本 crate，本判据不声称测到解码/播放。
    let limits = ParseLimits::default();
    for (root_rel, entry_rel) in [
        ("karoryfer-bear-sax", "Programs/1-solo-mono.sfz"),
        (
            "karoryfer-bigcat-cello",
            "Programs/01- Bowed (velocity layer).sfz",
        ),
        (
            "karoryfer-meatbass",
            "Meatbass/Programs/01_arco_modwheel.sfz",
        ),
        ("karoryfer-string-cyborgs", "Programs/Blackheart.sfz"),
    ] {
        let entry_path = corpus_root().join(root_rel).join(entry_rel);
        let base_dir = entry_path.parent().expect("入口必有父目录");
        let entry_name = entry_path
            .file_name()
            .expect("入口必有文件名")
            .to_string_lossy()
            .into_owned();
        let resolver = IncludeResolver::new(base_dir, limits)
            .unwrap_or_else(|error| panic!("{entry_rel} 的解析器构造失败：{error}"));
        let sources = resolver
            .resolve(&entry_name)
            .unwrap_or_else(|error| panic!("{entry_rel} 的 include 展开失败：{error}"));
        let instrument = parse_sources(&sources, &limits)
            .unwrap_or_else(|error| panic!("{entry_rel} 修复后必须可装载：{error}"));
        assert!(
            !instrument.regions().is_empty(),
            "{entry_rel} 必须归约出 region"
        );
        let notes: Vec<&Warning> = instrument
            .warnings()
            .iter()
            .filter(|warning| {
                matches!(
                    warning,
                    Warning::ClampedModulationDepth { opcode, .. } if opcode == "pan_oncc10"
                )
            })
            .collect();
        assert_eq!(
            notes.len(),
            1,
            "{entry_rel} 必须恰一条点名 `pan_oncc10` 的钳位告警\
             （同一 `<global>` 声明被上千 region 重读，去重后只记第一次），实得 {:?}",
            instrument.warnings()
        );
        println!(
            "[oncc-clamp] {entry_rel}: regions={} clamp_notes={}",
            instrument.regions().len(),
            notes.len()
        );
    }
}

/// 取唯一的指定目标调制项（声像族建模补全片用；`pan` 版见 `the_pan_mod`）。
fn the_stereo_mod(text: &str, target: ModTarget) -> CcMod {
    let instrument = parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"));
    let mods: Vec<&CcMod> = instrument.regions()[0]
        .cc_modulations()
        .iter()
        .filter(|item| item.target == target)
        .collect();
    assert_eq!(mods.len(), 1, "`{text}` 必须恰一条 {target:?} 调制项");
    *mods[0]
}

#[test]
fn position_and_width_oncc_depths_clamp_like_pan() {
    // 声像族两名新目标与 `pan` 同口径：深度越界装载期钳位 + 点名告警。
    let position = the_stereo_mod(
        "<region> sample=a.wav position_oncc10=200",
        ModTarget::Position,
    );
    assert_eq!(position.cc, 10);
    assert_eq!(position.delta, 100.0, "position 深度必须落到值域上界 100");
    assert_eq!(
        clamp_notes("<region> sample=a.wav position_oncc10=200"),
        vec![Warning::ClampedModulationDepth {
            line: 1,
            opcode: "position_oncc10".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }]
    );
    let width = the_stereo_mod("<region> sample=a.wav width_oncc10=-200", ModTarget::Width);
    assert_eq!(width.delta, -100.0, "width 深度必须落到值域下界 -100");
    // 域内深度无告警。
    assert!(clamp_notes("<region> sample=a.wav width_oncc10=-100").is_empty());
}

#[test]
fn position_and_width_evaluate_with_baked_cc() {
    // position：基准 10 + `position_oncc1=50`，CC 满 ⇒ 60，CC 为 0 ⇒ 10，
    // probe 缺席 ⇒ 基准原样（与 `pan_value_with_baked` 同口径）。
    let instrument = parse_text(
        "<region> sample=a.wav position=10 position_oncc1=50",
        &ParseLimits::default(),
    )
    .unwrap();
    let region = &instrument.regions()[0];
    let baked = bake_cc_mods(region.cc_modulations(), Curve::built_in);
    assert_eq!(
        position_value_with_baked(region.position, &baked, Some(&|_| 127)),
        60.0
    );
    assert_eq!(
        position_value_with_baked(region.position, &baked, Some(&|_| 0)),
        10.0
    );
    assert_eq!(
        position_value_with_baked(region.position, &baked, None),
        10.0
    );
    // width：缺省基准 100 + `width_oncc1=-100`，CC 满 ⇒ 0（单声道化点）。
    let instrument = parse_text(
        "<region> sample=a.wav width_oncc1=-100",
        &ParseLimits::default(),
    )
    .unwrap();
    let region = &instrument.regions()[0];
    assert_eq!(region.width, 100.0, "width 缺省必须是 100（立体声原样）");
    let baked = bake_cc_mods(region.cc_modulations(), Curve::built_in);
    assert_eq!(
        width_value_with_baked(region.width, &baked, Some(&|_| 127)),
        0.0
    );
    // 求值和越界钳回：80 + 50 ⇒ 100（不是 130、不是回绕）。
    let instrument = parse_text(
        "<region> sample=a.wav position=80 position_oncc1=50",
        &ParseLimits::default(),
    )
    .unwrap();
    let region = &instrument.regions()[0];
    let baked = bake_cc_mods(region.cc_modulations(), Curve::built_in);
    assert_eq!(
        position_value_with_baked(region.position, &baked, Some(&|_| 127)),
        100.0
    );
}
