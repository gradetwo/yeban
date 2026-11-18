//! ⭐ **声像族静态值钳位判据**（Wang 2026-11-18 裁决）。
//!
//! # 裁决与本判据在防什么
//!
//! 旧口径：静态 `pan` 在装载层**没有**值域校验（原样透传、求值期才
//! 钳位）；`position` / `width` 未建模、按未知 opcode 忽略。裁决（与
//! 同日调制深度钳位同一条规矩，判据在 `tests/modulation_depth_clamp.rs`）：
//! 声像族三个**静态** opcode（`pan` / `position` / `width`，规范值域
//! 均为 `-100..=100`）越界时，**装载期钳位到值域边界 ＋ 经装载报告的
//! 告警通道 [`Warning::ClampedStaticValue`] 响亮记录、不拒载**；告警按
//! (opcode, 原值, 钳位值) 跨 region 去重。求值期本来就钳位 ⇒ 听感零
//! 变化，本判据钉的是装载报告与存储值的口径，不是听感。
//!
//! ⚠ 边界：声像族**之外**的静态值维持严格报错（如 `amp_veltrack=101`
//! 仍 `Err`）；三个参数的调制深度走 `ClampedModulationDepth`，不在
//! 本判据；`position` / `width` 在本装载器尚无存储与求值语义，本判据
//! 对它们钉的是**装载期值域执法**（越界数值告警、畸形写法维持忽略、
//! ⛔ 不新增拒载）。
//!
//! ⭐ **追加修订段（声像族建模补全片，Wang 2026-11-18）—— 被修订的是
//! 上一段里「`position` / `width` 尚无存储与求值语义、畸形写法维持
//! 忽略」那一支**（⛔ 原文一字未删）：两参数现已建模（`Region` 存储 ＋
//! 调制目标 ＋ 引擎立体声级），畸形 / 非有限写法改走与 `pan` 相同的
//! 明确 `Err`（`InvalidFloat` / `NonFiniteFloat`，「默认不能静默」）；
//! 判据 3 顺带补钉存储值、判据 6 的 `position` / `width` 支按此改钉。
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | 1 | `pan=200` 可装载、存储值落 100、告警载荷逐字可查 | `Region::pan` ＋ `Warning::ClampedStaticValue` |
//! | 2 | `pan=-250` 钳到 −100 | 同上 |
//! | 3 | `position=150` / `width=-250` 可装载且各出一条点名告警 | `Warning::ClampedStaticValue` |
//! | 4 | 域内值（含两端点）原样、无告警 | `warnings()` 为空 ＋ `Region::pan` |
//! | 5 | 同一声明跨 region 只告警一次；不同原值各记一条；被覆盖的越界值不告警 | 告警条数与载荷 |
//! | 6 | 声像族之外的严格口径未被误伤；`pan` 本体畸形仍 `Err`；`position` / `width` 畸形改与 `pan` 同口径 `Err`（建模补全片改钉） | `SfzError` 变体 ＋ 装载成败 |
//! | 7 | 钳位后的基准与调制增量叠加，求值仍落在 `-100..=100`（无双重钳位怪值） | `pan_value_with_baked` |
//! | 8 | 真实文件回归：bear-sax（静态 `pan` 域内 ＋ `pan_oncc10=200`）与 virtuosity-drums（静态 `width=0`）装载成功、region 数与基线一致、无新增静态钳位告警 | include 展开 ＋ 解析归约（**装载级**；样本解码不在本 crate） |

use std::path::{Path, PathBuf};

use yeban_sfz::curve::Curve;
use yeban_sfz::modulation::{bake_cc_mods, pan_value_with_baked};
use yeban_sfz::{
    IncludeResolver, Instrument, ParseLimits, SfzError, Warning, parse_sources, parse_text,
};

/// 装载一份文本（必须成功）。
fn load(text: &str) -> Instrument<'_> {
    parse_text(text, &ParseLimits::default())
        .unwrap_or_else(|error| panic!("`{text}` 必须可装载：{error}"))
}

/// 一份文本里全部静态钳位告警（顺序即装载报告顺序）。
fn static_clamp_notes(text: &str) -> Vec<Warning> {
    load(text)
        .warnings()
        .iter()
        .filter(|warning| matches!(warning, Warning::ClampedStaticValue { .. }))
        .cloned()
        .collect()
}

#[test]
fn pan_above_the_range_is_clamped_to_100_at_load_with_a_warning() {
    let text = "<region> sample=a.wav pan=200";
    let instrument = load(text);
    assert_eq!(
        instrument.regions()[0].pan,
        100.0,
        "存储的基准 `pan` 必须在装载期就落到值域上界 100"
    );
    let notes = static_clamp_notes(text);
    assert_eq!(
        notes,
        vec![Warning::ClampedStaticValue {
            line: 1,
            opcode: "pan".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }],
        "告警必须在装载报告里逐字可查（行号 / opcode / 原值 / 钳位值）"
    );
    assert_eq!(
        notes[0].to_string(),
        "line 1: `pan` value 200 out of range, clamped to 100"
    );
    // 静态钳位⛔ 不许冒充调制钳位：装载报告**全量**逐字等于这一条
    // `ClampedStaticValue` ⇒ 不得混入任何 `ClampedModulationDepth`。
    assert_eq!(
        instrument.warnings(),
        &[Warning::ClampedStaticValue {
            line: 1,
            opcode: "pan".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }]
    );
}

#[test]
fn pan_below_the_range_is_clamped_to_minus_100_at_load() {
    let text = "<region> sample=a.wav pan=-250";
    assert_eq!(load(text).regions()[0].pan, -100.0);
    assert_eq!(
        static_clamp_notes(text),
        vec![Warning::ClampedStaticValue {
            line: 1,
            opcode: "pan".to_string(),
            value: "-250".to_string(),
            clamped: "-100".to_string(),
        }]
    );
}

#[test]
fn position_and_width_out_of_range_warn_and_still_load() {
    // 建模补全片后判据加强：除装载期值域执法（可装载 ＋ 点名告警）外，
    // 钳位值即存储值（`Region::position` / `Region::width`）。
    let position = load("<region> sample=a.wav position=150");
    assert_eq!(position.regions().len(), 1);
    assert_eq!(
        position.regions()[0].position, 100.0,
        "钳位后的 100 必须即存储的 position 基准"
    );
    assert_eq!(
        load("<region> sample=a.wav width=-250").regions()[0].width, -100.0,
        "钳位后的 -100 必须即存储的 width 基准"
    );
    assert_eq!(
        position.warnings(),
        &[Warning::ClampedStaticValue {
            line: 1,
            opcode: "position".to_string(),
            value: "150".to_string(),
            clamped: "100".to_string(),
        }]
    );
    assert_eq!(
        static_clamp_notes("<region> sample=a.wav width=-250"),
        vec![Warning::ClampedStaticValue {
            line: 1,
            opcode: "width".to_string(),
            value: "-250".to_string(),
            clamped: "-100".to_string(),
        }]
    );
}

#[test]
fn in_range_stereo_values_produce_no_warning() {
    // 两端点与域内值都**不**告警（钳位只发生在真的越界时），
    // `pan` 存储值原样。
    for (text, want) in [
        ("<region> sample=a.wav pan=100", 100.0),
        ("<region> sample=a.wav pan=-100", -100.0),
        ("<region> sample=a.wav pan=0", 0.0),
        ("<region> sample=a.wav pan=42.5", 42.5),
    ] {
        let instrument = load(text);
        assert_eq!(
            instrument.regions()[0].pan,
            want,
            "`{text}` 的 pan 必须原样"
        );
        assert!(
            instrument.warnings().is_empty(),
            "`{text}` 在域内不得产生任何告警，实得 {:?}",
            instrument.warnings()
        );
    }
    for text in [
        "<region> sample=a.wav position=100",
        "<region> sample=a.wav position=-100",
        "<region> sample=a.wav width=100",
        "<region> sample=a.wav width=0",
    ] {
        let instrument = load(text);
        assert!(
            instrument.warnings().is_empty(),
            "`{text}` 在域内不得产生任何告警，实得 {:?}",
            instrument.warnings()
        );
    }
}

#[test]
fn the_same_declaration_warns_once_across_regions() {
    // 同一条 `<global>` 声明被每个 region 经作用域继承重读：同一事实
    // 只记第一次出现（行号 ＝ 第一条 region 的段头行）。
    let text =
        "<global> pan=200\n<region> sample=a.wav\n<region> sample=b.wav\n<region> sample=c.wav";
    let instrument = load(text);
    assert_eq!(instrument.regions().len(), 3);
    assert!(
        instrument
            .regions()
            .iter()
            .all(|region| region.pan == 100.0),
        "每条 region 的存储 pan 都必须是钳位后的 100"
    );
    assert_eq!(
        static_clamp_notes(text),
        vec![Warning::ClampedStaticValue {
            line: 2,
            opcode: "pan".to_string(),
            value: "200".to_string(),
            clamped: "100".to_string(),
        }],
        "跨 region 去重后必须恰一条"
    );
    // `width` 同口径去重。
    let width_text = "<global> width=300\n<region> sample=a.wav\n<region> sample=b.wav";
    assert_eq!(static_clamp_notes(width_text).len(), 1);
    // 不同原值是不同事实 ⇒ 各记一条（去重键含原值）。
    let two = "<region> sample=a.wav pan=200\n<region> sample=b.wav pan=300";
    assert_eq!(static_clamp_notes(two).len(), 2);
    // 被 region 级声明覆盖掉的越界 `<global>` 值从未生效 ⇒ 不告警
    // （与调制深度「只为归约胜出的取值发出」同一口径）。
    let overridden = "<global> pan=200\n<region> sample=a.wav pan=30";
    let instrument = load(overridden);
    assert_eq!(instrument.regions()[0].pan, 30.0);
    assert!(
        instrument.warnings().is_empty(),
        "被覆盖的越界值不得告警，实得 {:?}",
        instrument.warnings()
    );
}

#[test]
fn strictness_outside_the_pan_family_is_unchanged() {
    // 声像族之外的静态值越界维持明确 `Err` —— 即使同一份文件里有一条
    // 已被钳位的声像值。
    let outcome = parse_text(
        "<region> sample=a.wav pan=200 amp_veltrack=101",
        &ParseLimits::default(),
    );
    assert!(
        matches!(
            outcome,
            Err(SfzError::FloatOutOfRange { ref opcode, .. }) if opcode == "amp_veltrack"
        ),
        "静态 `amp_veltrack=101` 必须仍是 FloatOutOfRange，实得 {outcome:?}"
    );
    // `pan` 本体的畸形 / 非有限维持现行错误（不是「越界」，⛔ 不钳位放行）。
    assert!(matches!(
        parse_text("<region> sample=a.wav pan=abc", &ParseLimits::default()),
        Err(SfzError::InvalidFloat { .. })
    ));
    assert!(matches!(
        parse_text("<region> sample=a.wav pan=inf", &ParseLimits::default()),
        Err(SfzError::NonFiniteFloat { .. })
    ));
    // ⭐ 建模补全片改钉（原钉「未建模、畸形维持忽略」见文件头修订段）：
    // `position` / `width` 已建模 ⇒ 畸形 / 非有限与 `pan` 同口径明确
    // `Err`，⛔ 不再静默忽略。
    assert!(matches!(
        parse_text("<region> sample=a.wav width=abc", &ParseLimits::default()),
        Err(SfzError::InvalidFloat { .. })
    ));
    assert!(matches!(
        parse_text("<region> sample=a.wav position=inf", &ParseLimits::default()),
        Err(SfzError::NonFiniteFloat { .. })
    ));
}

#[test]
fn a_clamped_base_plus_modulation_stays_in_range_at_evaluation() {
    // 装载期钳位后的基准（100）与调制增量叠加：求值本体对和值另有
    // 一次同域钳位 ⇒ 结果只会收敛到域内，⛔ 不产生双重钳位怪值。
    let down = load("<region> sample=a.wav pan=200 pan_oncc10=-100");
    let region = &down.regions()[0];
    assert_eq!(region.pan, 100.0, "基准必须是装载期钳位后的 100");
    let baked = bake_cc_mods(region.cc_modulations(), Curve::built_in);
    assert_eq!(
        pan_value_with_baked(region.pan, &baked, Some(&|_| 127)),
        0.0,
        "100 ＋ (−100 × 曲线 0 满量程 1.0) 必须恰 0"
    );
    let up = load("<region> sample=a.wav pan=200 pan_oncc10=100");
    let region = &up.regions()[0];
    let baked = bake_cc_mods(region.cc_modulations(), Curve::built_in);
    assert_eq!(
        pan_value_with_baked(region.pan, &baked, Some(&|_| 127)),
        100.0,
        "100 ＋ 100 的和必须在求值期钳回 100（不是 200、不是回绕值）"
    );
    // 两份文件的调制深度都在域内 ⇒ 装载报告**全量**逐字等于那条静态
    // 钳位告警（没有调制钳位告警混入）。
    for instrument in [&down, &up] {
        assert_eq!(
            instrument.warnings(),
            &[Warning::ClampedStaticValue {
                line: 1,
                opcode: "pan".to_string(),
                value: "200".to_string(),
                clamped: "100".to_string(),
            }]
        );
    }
}

/// 语料根（相对 crate 根：`crates/yeban-sfz/../../assets/samples`）。
fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/samples")
}

/// 装载一份真实语料入口（include 展开 ＋ 解析归约，装载级），在
/// sources 存活期间把 [`Instrument`] 交给 `check` 检查（`Instrument`
/// 借用 sources 文本，不能搬出本帧）。
fn with_corpus_entry<R>(
    root_rel: &str,
    entry_rel: &str,
    check: impl FnOnce(&Instrument<'_>) -> R,
) -> R {
    let limits = ParseLimits::default();
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
        .unwrap_or_else(|error| panic!("{entry_rel} 必须可装载：{error}"));
    check(&instrument)
}

#[test]
fn real_instruments_with_in_range_stereo_values_gain_no_new_warnings() {
    // 基线（2026-11-18 在未改动的 main `b6a3751fc` 上实测）：bear-sax
    // 入口 1551 条 region ＋ 恰 1 条 `pan_oncc10` 调制钳位告警（上一线）；
    // virtuosity-drums 入口 1676 条 region ＋ 0 条告警。两款的静态声像
    // 值都在域内（bear-sax 用静态 `pan=`、virtuosity 用 `width=0`）⇒
    // 本线之后 region 数必须不变、静态钳位告警必须为 0、既有告警不变。
    with_corpus_entry("karoryfer-bear-sax", "Programs/1-solo-mono.sfz", |bear| {
        assert_eq!(
            bear.regions().len(),
            1551,
            "bear-sax 的 region 数必须与基线一致"
        );
        assert!(
            !bear
                .warnings()
                .iter()
                .any(|warning| matches!(warning, Warning::ClampedStaticValue { .. })),
            "bear-sax 的静态 pan 在域内，不得新增静态钳位告警，实得 {:?}",
            bear.warnings()
        );
        assert_eq!(
            bear.warnings()
                .iter()
                .filter(|warning| matches!(warning, Warning::ClampedModulationDepth { .. }))
                .count(),
            1,
            "上一线的 `pan_oncc10` 调制钳位告警必须仍恰 1 条"
        );
    });

    with_corpus_entry("virtuosity-drums", "Programs/01-basic-kit.sfz", |drums| {
        assert_eq!(
            drums.regions().len(),
            1676,
            "virtuosity-drums 的 region 数必须与基线一致"
        );
        assert!(
            drums.warnings().is_empty(),
            "virtuosity-drums 基线 0 告警、本线后仍须 0，实得 {:?}",
            drums.warnings()
        );
    });
}

#[test]
fn position_and_width_defaults_are_the_neutral_values() {
    // ⭐ 建模补全片判据：没写 `position` / `width` 的 region，存储值必须是
    // 中性缺省 —— `position` 0（居中）、`width` 100（立体声原样不变；
    // sfizz `Defaults.cpp` 与 sfzformat.com 的 width 页共同钉死，⛔ 不是 0）。
    let instrument = load("<region> sample=a.wav");
    assert_eq!(instrument.regions()[0].position, 0.0);
    assert_eq!(instrument.regions()[0].width, 100.0);
    // 显式写中性值同样无告警（域内）。
    let explicit = load("<region> sample=a.wav position=0 width=100");
    assert_eq!(explicit.regions()[0].position, 0.0);
    assert_eq!(explicit.regions()[0].width, 100.0);
    assert!(explicit.warnings().is_empty());
}

#[test]
fn position_and_width_follow_the_pan_scope_inheritance() {
    // 作用域继承与 `pan` 同一条链：global 被 group 覆盖、group 被 region 覆盖。
    let text = "<global> width=10 position=-20\n<group> width=30\n<region> sample=a.wav position=5";
    let instrument = load(text);
    assert_eq!(instrument.regions()[0].width, 30.0, "group 的 width 必须赢过 global");
    assert_eq!(instrument.regions()[0].position, 5.0, "region 的 position 必须赢过 global");
    // 只在 global 声明时原样继承。
    let inherited = load("<global> width=-40 position=25\n<region> sample=a.wav");
    assert_eq!(inherited.regions()[0].width, -40.0);
    assert_eq!(inherited.regions()[0].position, 25.0);
}
