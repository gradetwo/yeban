//! `ADR-0038`（审计缺口 `G8f` 的音域轴）：`yeban_propose_section` 的
//! `melodyLower` / `melodyUpper` —— 旋律声部的音高窗口从 theory 的出厂常量
//! `MelodyConstraints::DEFAULT`（48 / 84）换成可给的两个入参，缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **MIDI 音高值**（`0..=127`），⛔ 不是"第几个八度"、⛔ 不是半音偏移量。
//! 本票⛔ 重新发明"哪些音算这个调的音"（那是 `ADR-0028` 之外 theory 的口径），也
//! ⛔ 预先判"窗口里有没有音阶音" —— 那是调式 × 音域的可行性函数，唯一来源是 theory 的
//! `NoFeasibleVoicing`（判据 5 把它钉成"原样转发"）。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与
//! `ParamSpec::accepts("integer")`），纯领域层量不到 ⇒ 本文件把它写成
//! [`melody_register_is_declared_on_both_param_surfaces`] 里"`json_type` 必须是 integer"
//! （与 `ADR-0036` / `ADR-0037` 判据 5 的同一条分工）。
//!
//! ⚠ **力度不在本票的对齐范围内**（如实写在判据 2 里）：旋律的力度按 `chord_tone` 取
//! 阶梯两档，而 `chord_tone` 是**音高**的函数 ⇒ 收窄音域会改变哪些音是和弦音。
//! 因此判据 2 只对齐 id / 起点 / 时值（网格量，与音高无关），对力度只要求
//! "仍落在阶梯的两档里"，⛔ 断言逐条相等（那是把另一票的语义按在这里）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::{MELODY_LOWER_BOUND, MELODY_UPPER_BOUND};

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4（⇒ 缺省网格 16）且登记了旋律走向。
const STYLE: &str = "synthwave";
/// 夹具的调：C 自然小调 ⇒ 音级集合不含 `C#/Db`（判据 5 用它构造不可行窗口）。
const KEY: &str = "C minor";
/// 单音窗口且**可行**（`C4` ＝该调主音，实测 theory 返回 `Ok` 并把整条旋律压成这一音）。
const FEASIBLE_SINGLE: u64 = 60;
/// 单音窗口且**不可行**（`C#4` 不在 C 自然小调里，实测 theory 返回 `NoFeasibleVoicing`）。
const INFEASIBLE_SINGLE: u64 = 61;
/// 缺省阶梯的两档（`ADR-0037` 的缺省值；本文件的夹具不给力度键）。
const LADDER: [u8; 2] = [96, 112];

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律，两键都不给）。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    include_melody: bool,
    melody_lower: Option<u64>,
    melody_upper: Option<u64>,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_melody: true,
            melody_lower: None,
            melody_upper: None,
        }
    }

    fn lower(mut self, value: Option<u64>) -> Self {
        self.melody_lower = value;
        self
    }

    fn upper(mut self, value: Option<u64>) -> Self {
        self.melody_upper = value;
        self
    }

    fn material_source(mut self, value: Option<&'static str>) -> Self {
        self.material_source = value;
        self
    }

    fn melody(mut self, value: bool) -> Self {
        self.include_melody = value;
        self
    }
}

fn try_plan(given: Given) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        STYLE,
        BARS,
        Some(KEY),
        PlanOptions {
            material_source: given.material_source,
            progression: None,
            include_drums: Some(true),
            include_melody: Some(given.include_melody),
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
            melody_lower: given.melody_lower,
            melody_upper: given.melody_upper,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（lower={:?} upper={:?}）必须成功: {fault:?}",
            given.melody_lower, given.melody_upper
        )
    })
}

/// 按**声部名**索引的音符表（声部名取 `"{section} · {part}"` 的后半）。
fn voices(section_plan: &SectionPlan) -> BTreeMap<String, BTreeMap<String, Note>> {
    let mut map: BTreeMap<String, BTreeMap<String, Note>> = BTreeMap::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else { continue };
        let ClipContent::Midi { notes } = &clip.content else {
            continue;
        };
        let part = clip
            .name
            .rsplit(" · ")
            .next()
            .unwrap_or(clip.name.as_str())
            .to_owned();
        let table = map.entry(part).or_default();
        for note in notes.values() {
            table.insert(
                note.id.to_canonical_string(),
                (
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.velocity,
                ),
            );
        }
    }
    map
}

/// 某声部的音高多重集（去重后升序）。
fn pitches(table: &BTreeMap<String, Note>) -> Vec<u8> {
    let mut values: Vec<u8> = table.values().map(|note| note.2).collect();
    values.sort_unstable();
    values.dedup();
    values
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0038` D4：沿用既有形状词 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
fn assert_shape_reject(fault: &BuildFault, expected: &str) {
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "{expected} 必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    let payload = data(fault);
    assert!(
        payload.get("reason").is_none(),
        "{expected}：载荷⛔ 带 `reason` 键（否则本票会撑大故障词表）: {payload}"
    );
}

/// ⭐ 判据 1（`ADR-0038` D2）：**两键都不给 ≡ 给 `48`/`84`**，整个规划逐字节相同
/// （`SectionPlan: PartialEq`）；且缺省旋律音高全部落在 theory 出厂窗口内。
#[test]
fn absent_register_is_byte_identical_to_the_published_window() {
    let absent = plan(Given::theory());
    let explicit = plan(
        Given::theory()
            .lower(Some(u64::from(MELODY_LOWER_BOUND)))
            .upper(Some(u64::from(MELODY_UPPER_BOUND))),
    );
    assert_eq!(
        absent, explicit,
        "两个键缺席必须逐字节等于给 `{{\"melodyLower\": {MELODY_LOWER_BOUND}, \"melodyUpper\": {MELODY_UPPER_BOUND}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let melody = &table["Melody"];
    let values = pitches(melody);
    for pitch in &values {
        assert!(
            (MELODY_LOWER_BOUND..=MELODY_UPPER_BOUND).contains(pitch),
            "缺省旋律音高 `{pitch}` 必须落在出厂窗口 {MELODY_LOWER_BOUND}..={MELODY_UPPER_BOUND}: {values:?}"
        );
    }
    // 夹具必须真的产出一条有内容的旋律（否则后面每条判据都是空判据）。
    assert!(values.len() > 1, "夹具的缺省旋律必须有多个音高: {values:?}");
}

/// ⭐ 判据 2（D1 的映射面 + "只作用于旋律"）：把窗口收到 `FEASIBLE_SINGLE..=FEASIBLE_SINGLE`
/// 与收到 `48..=55` ⇒
/// * 旋律的**音符 id 集合、起点、时值**与缺省跑逐条相同（音域⛔ 不筛点、⛔ 不改时值）；
/// * 每条旋律音符的音高都落在请求窗口内（正例真的被消费）；
/// * 和声与鼓两条路径与缺省跑**逐字节**相同（音域⛔ 溢出到别的声部）；
/// * 力度仍然只取阶梯的两档（⚠ 不逐条对齐 —— `chord_tone` 是音高的函数，见文件头）。
#[test]
fn a_narrow_register_moves_only_the_melody_pitches() {
    let base = plan(Given::theory());
    let base_table = voices(&base);

    for (low, high) in [
        (FEASIBLE_SINGLE, FEASIBLE_SINGLE),
        (48, 55),
        (80, u64::from(MELODY_UPPER_BOUND)),
    ] {
        let shifted = plan(Given::theory().lower(Some(low)).upper(Some(high)));
        let shifted_table = voices(&shifted);
        assert_eq!(
            base_table.keys().collect::<Vec<_>>(),
            shifted_table.keys().collect::<Vec<_>>(),
            "窗口 {low}..={high}：声部集合必须不变"
        );
        for part in preset_parts(STYLE).expect("夹具必须在册") {
            assert_eq!(
                base_table[part], shifted_table[part],
                "窗口 {low}..={high}：和声声部 `{part}` 必须与缺省跑逐条相同（音域只作用于旋律）"
            );
        }
        assert_eq!(
            base_table["Drums"], shifted_table["Drums"],
            "窗口 {low}..={high}：鼓声部必须与缺省跑逐条相同（鼓的音高是 GM 键）"
        );
        let base_melody = &base_table["Melody"];
        let shifted_melody = &shifted_table["Melody"];
        assert_eq!(
            base_melody.keys().collect::<Vec<_>>(),
            shifted_melody.keys().collect::<Vec<_>>(),
            "窗口 {low}..={high}：旋律的音符 id 集合必须⛔ 不受音域影响（不筛点）"
        );
        let mut moved_ids: Vec<&String> = Vec::new();
        for (id, (start, duration, pitch, velocity)) in base_melody {
            let (shift_start, shift_duration, shift_pitch, shift_velocity) = shifted_melody[id];
            assert_eq!(
                (start, duration),
                (&shift_start, &shift_duration),
                "窗口 {low}..={high}：旋律音符 `{id}` 的起点与时值不得因音域改变"
            );
            assert!(
                (u8::try_from(low).expect("low 在 u8 内")
                    ..=u8::try_from(high).expect("high 在 u8 内"))
                    .contains(&shift_pitch),
                "窗口 {low}..={high}：旋律音符 `{id}` 的音高 {shift_pitch} 必须落在窗口内"
            );
            assert!(
                LADDER.contains(&shift_velocity),
                "窗口 {low}..={high}：旋律音符 `{id}` 的力度 {shift_velocity} 必须仍取阶梯两档 {LADDER:?}"
            );
            if *pitch != shift_pitch {
                moved_ids.push(id);
            }
        }
        assert!(
            !moved_ids.is_empty(),
            "窗口 {low}..={high}：必须真的改变至少一条旋律音符的音高（否则正例是空判据）: {:?}",
            moved_ids
        );
    }

    // 单音窗口：整条旋律必须压成那一个音高（theory 实测形状）。
    let single = plan(
        Given::theory()
            .lower(Some(FEASIBLE_SINGLE))
            .upper(Some(FEASIBLE_SINGLE)),
    );
    assert_eq!(
        pitches(&voices(&single)["Melody"]),
        vec![u8::try_from(FEASIBLE_SINGLE).expect("常量在 u8 内")],
        "`lower == upper` 必须合法并把整条旋律压到那一个音高"
    );
}

/// ⭐ 判据 3（D1/D3 的形状面）：越界（`128` / 超出 `u8` 宽度）⇒ 点名是哪个键；
/// 倒序 ⇒ `lower > upper` 拒绝并回显两值；`lower == upper` 合法。
#[test]
fn out_of_range_and_inverted_are_rejected_naming_the_key() {
    for key in ["melodyLower", "melodyUpper"] {
        for raw in [128u64, 1_000, u64::from(u8::MAX) + 1] {
            let given = if key == "melodyLower" {
                Given::theory().lower(Some(raw))
            } else {
                Given::theory().upper(Some(raw))
            };
            let fault = try_plan(given).expect_err("越界不得静默夹紧");
            assert_shape_reject(&fault, &format!("`{key}` 越界 ({raw})"));
            let payload = data(&fault);
            assert_eq!(
                payload["field"],
                json!(key),
                "载荷必须点名是哪个键: {payload}"
            );
            assert_eq!(
                payload["value"],
                json!(raw),
                "载荷必须回显被拒的取值: {payload}"
            );
            assert_eq!(
                payload["range"],
                json!([0, 127]),
                "载荷必须给出 MIDI 音高的界: {payload}"
            );
        }
    }
    // 倒序：音高窗口⛔ 没有"倒着开"的语义。
    let fault = try_plan(Given::theory().lower(Some(72)).upper(Some(60)))
        .expect_err("倒序窗口（lower > upper）必须被拒绝，⛔ 当成合法输入");
    assert_shape_reject(&fault, "倒序音域");
    let payload = data(&fault);
    assert_eq!(payload["field"], json!("melodyLower"), "载荷: {payload}");
    assert_eq!(payload["melodyLower"], json!(72), "载荷: {payload}");
    assert_eq!(payload["melodyUpper"], json!(60), "载荷: {payload}");
    assert!(
        payload["hint"]
            .as_str()
            .is_some_and(|text| text.contains("melodyLower") && text.contains("melodyUpper")),
        "载荷的 `hint` 必须同时点名两个键（否则调用方不知道该改哪一个）: {payload}"
    );
}

/// ⭐ 判据 4（D3 的消费面）：两键⛔ 脱离 `materialSource:"theory"`，也⛔ 脱离
/// `includeMelody: true` —— 与力度那一票不同，音域**没有**"恒消费"的路径。
#[test]
fn register_keys_require_theory_source_and_a_melody_part() {
    for material_source in [None, Some("project")] {
        for which in ["melodyLower", "melodyUpper"] {
            let given = if which == "melodyLower" {
                Given::theory()
                    .material_source(material_source)
                    .lower(Some(60))
            } else {
                Given::theory()
                    .material_source(material_source)
                    .upper(Some(72))
            };
            let fault = try_plan(given).expect_err(&format!(
                "`{which}` 不得脱离 materialSource:\"theory\" 单独给"
            ));
            assert_shape_reject(&fault, &format!("`{which}` 脱离 theory"));
            assert!(
                data(&fault)["hint"]
                    .as_str()
                    .is_some_and(|text| text.contains("materialSource")),
                "载荷必须指出要补 materialSource: {}",
                data(&fault)
            );
        }
    }
    // 有 theory 但没有旋律声部 ⇒ 同样必须拒（键缺省 `includeMelody:false`）。
    for which in ["melodyLower", "melodyUpper"] {
        let given = if which == "melodyLower" {
            Given::theory().lower(Some(60))
        } else {
            Given::theory().upper(Some(72))
        };
        let fault = try_plan(given.melody(false))
            .expect_err(&format!("`{which}` 不得在没有旋律声部时静默通过"));
        assert_shape_reject(&fault, &format!("`{which}` 脱离 includeMelody"));
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("includeMelody")),
            "载荷必须指出要补 includeMelody: {}",
            data(&fault)
        );
    }
    // 正例：theory + 旋律 ⇒ 窗口必须真的生效（判据 2 已逐条对齐，这里只核一条）。
    let narrow = plan(Given::theory().lower(Some(48)).upper(Some(52)));
    let values = pitches(&voices(&narrow)["Melody"]);
    assert!(
        values.iter().all(|pitch| (48..=52).contains(pitch)),
        "窗口 48..=52 必须真的把旋律压进那五个半音: {values:?}"
    );
}

/// ⭐ 判据 5（D3/D4 的转发面）：窗口里**没有**该调的音 ⇒ 由 theory 判定，本层
/// 原样转发（`theoryStep` 指向旋律阶段），⛔ 本层抄一张"调式 × 音域"可行性表，
/// 也⛔ 静默产出空旋律。载荷⛔ 带 `reason` 键 ⇒ 故障词表不变。
#[test]
fn an_unfeasible_window_is_forwarded_from_theory_not_pre_validated() {
    let fault = try_plan(
        Given::theory()
            .lower(Some(INFEASIBLE_SINGLE))
            .upper(Some(INFEASIBLE_SINGLE)),
    )
    .expect_err("窗口内没有音阶音必须被拒绝（⛔ 静默产出空旋律）");
    assert_shape_reject(&fault, "不可行窗口");
    let payload = data(&fault);
    assert_eq!(
        payload["theoryStep"],
        json!("melody_over_chords"),
        "必须把失败归到旋律生成阶段（转发 theory 的判定）: {payload}"
    );
    assert_eq!(
        payload["stylePreset"],
        json!(STYLE),
        "载荷必须给出是哪个风格预设: {payload}"
    );
    assert!(
        payload["theoryError"].is_string(),
        "载荷必须带上 theory 的原文（⛔ 本层改写成一个新词）: {payload}"
    );
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON），且契约的 `default`
/// 与 theory 的出厂两界是**同一组数字**；类型必须是 integer（工具面的"非整数 / 负数"
/// 由这道类型闸承担 —— 见文件头）。
#[test]
fn melody_register_is_declared_on_both_param_surfaces() {
    let spec = TOOLS
        .iter()
        .find(|tool| tool.name == "yeban_propose_section")
        .expect("注册表里必须有 `yeban_propose_section`");
    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("仓库根目录");
    let text = std::fs::read_to_string(repo.join("schemas").join("mcp-tools.schema.json"))
        .expect("读契约 schema");
    let contract: Value = serde_json::from_str(&text).expect("契约必须是合法 JSON");
    let declared = &contract["definitions"]["DocumentedToolArguments"]["$defs"]["yeban_propose_section"]
        ["properties"];
    for (name, default) in [
        ("melodyLower", MELODY_LOWER_BOUND),
        ("melodyUpper", MELODY_UPPER_BOUND),
    ] {
        let param = spec
            .param(name)
            .unwrap_or_else(|| panic!("参数表必须声明 `{name}`"));
        assert_eq!(param.json_type, "integer", "`{name}` 必须是整数参数");
        assert!(!param.required, "缺省 = 出厂音域 ⇒ 必须可选");
        assert!(
            param.doc.contains("ADR-0038") && param.doc.contains("MIDI 音高值"),
            "参数说明必须写清单位与出处: {}",
            param.doc
        );
        let property = &declared[name];
        assert!(
            !property.is_null(),
            "契约 JSON 必须登记 `{name}` 的属性: {text}"
        );
        assert_eq!(property["type"], json!("integer"));
        assert_eq!(property["minimum"], json!(0), "`{name}` 的下界: {property}");
        assert_eq!(
            property["maximum"],
            json!(127),
            "`{name}` 的上界必须与 `MIDI_PITCH_MAX` 同一组数字: {property}"
        );
        assert_eq!(
            property["default"],
            json!(default),
            "`{name}` 的缺省必须与 theory 的出厂界同源（⛔ 两份数字）: {property}"
        );
    }
    // ⛔ 第三个键：`max_leap` 的单位不是音域，本票明文不开放（`ADR-0038` N2）。
    assert!(
        spec.param("melodyMaxLeap").is_none() && declared.get("melodyMaxLeap").is_none(),
        "`melodyMaxLeap` 属 `ADR-0038` 非范围 N2 ⇒ 两个声明面都⛔ 出现它"
    );
}
