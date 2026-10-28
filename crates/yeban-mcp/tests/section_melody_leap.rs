//! `ADR-0039`（`ADR-0038` 非范围 `N2` 的兑现票）：`yeban_propose_section` 的
//! `melodyMaxLeap` —— 旋律**相邻两音的跳进上界**从 theory 的出厂常量
//! `MelodyConstraints::DEFAULT.max_leap`（12）换成可给的入参，缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **半音距离**（`0..=127`），⛔ 不是 MIDI 音高值（那是 `ADR-0038` 的两键）、
//! ⛔ 不是八度号、⛔ 不是音域宽度。上界为什么是 `127`：音高全域是 `0..=127` ⇒ 两音的
//! 半音距离在任何输入下都不可能超过 127（`ADR-0039` D2），本层⛔ 另立一张"看起来合理"的表。
//!
//! ⛔ 在本层预判"这个上界在当前窗口里可不可行"（判据 5 把 `0` 钉成合法而⛔ 越界）：
//! 可行性唯一来源是 theory（`melody.rs` 明文"⛔ 静默放宽上界"），命中时原样转发 ——
//! 与 `ADR-0038` D3 对音域的处置是同一条理由，只是这条轴上⛔ 需要构造不可行用例
//! （窗口不变时任何上界都有解：最坏情况是同音反复）。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与
//! `ParamSpec::accepts("integer")`），纯领域层量不到 ⇒ 本文件把它写成判据 6 里
//! "`json_type` 必须是 integer"（与 `ADR-0036`／`ADR-0037`／`ADR-0038` 判据 5 同一条分工）。
//!
//! ⚠ **力度不在对齐范围内**（同 `ADR-0038` 的判据 2）：旋律力度按 `chord_tone` 取阶梯两档，
//! 而 `chord_tone` 是**音高**的函数 ⇒ 收紧跳进会改变哪些音是和弦音。因此这里只对齐
//! id / 起点 / 时值（与音高无关的网格量），对力度只要求"仍落在阶梯两档里"。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::MELODY_MAX_LEAP;

/// 夹具的小节数。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4（⇒ 缺省网格 16）且登记了旋律走向。
const STYLE: &str = "synthwave";
/// 夹具的调：C 自然小调（音阶内窗口足够宽 ⇒ 任何跳进上界都有解）。
const KEY: &str = "C minor";
/// 收紧到"级进为主"的上界（半音距离）：实测必须把旋律压窄到⛔ 出现三度以上的跳进。
const STEPWISE: u64 = 2;
/// 中间档的上界（半音距离）。
const NARROW: u64 = 5;
/// 缺省阶梯的两档（`ADR-0037` 的缺省值；本文件的夹具不给力度键）。
const LADDER: [u8; 2] = [96, 112];

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律，本键不给）。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    include_melody: bool,
    melody_max_leap: Option<u64>,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_melody: true,
            melody_max_leap: None,
        }
    }

    fn leap(mut self, value: Option<u64>) -> Self {
        self.melody_max_leap = value;
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
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: given.melody_max_leap,
            harmony_max_voice_jump: None,
            swing_permille: None,
            drum_backbeat: None,
            drum_style: None,
            melody_seed: None,
            beat_grouping: None,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（melodyMaxLeap={:?}）必须成功: {fault:?}",
            given.melody_max_leap
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

/// 某声部**按起点排序后**的相邻两音最大半音距离（单位 = 半音距离）。
///
/// ⚠ 必须先按起点排序再取相邻：音符表按 **id** 索引（id 是确定性哈希，⛔ 时间序），
/// 直接用迭代器顺序会把两条不相邻的音当作一对来量。
fn max_leap(table: &BTreeMap<String, Note>) -> u64 {
    let mut starts: Vec<(u64, u8)> = table.values().map(|note| (note.0, note.2)).collect();
    starts.sort_unstable();
    starts
        .windows(2)
        .map(|pair| pair[0].1.abs_diff(pair[1].1) as u64)
        .max()
        .unwrap_or(0)
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0039` D5 继承 `ADR-0038` D4 ⇒ `fault_vocabulary` 的词表与摘要不变）。
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

/// ⭐ 判据 1（`ADR-0039` D2）：**键缺席 ≡ 给出厂上界 `12`**，整个规划逐字节相同
/// （`SectionPlan: PartialEq`）；且缺省旋律的跳进恒 ≤ 出厂上界。
#[test]
fn absent_leap_is_byte_identical_to_the_published_bound() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().leap(Some(u64::from(MELODY_MAX_LEAP))));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"melodyMaxLeap\": {MELODY_MAX_LEAP}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let melody = &table["Melody"];
    let observed = max_leap(melody);
    assert!(
        observed <= u64::from(MELODY_MAX_LEAP),
        "缺省旋律的相邻跳进 {observed} 必须 ≤ 出厂上界 {MELODY_MAX_LEAP}"
    );
    // 夹具必须真的产出一条有内容的旋律（否则后面每条判据都是空判据）。
    assert!(
        pitches(melody).len() > 1,
        "夹具的缺省旋律必须有多个音高: {:?}",
        pitches(melody)
    );
}

/// ⭐ 判据 2（D1 的映射面 + "只作用于旋律"）：把上界收到 `NARROW` 与 `STEPWISE` ⇒
/// * 旋律的**音符 id 集合、起点、时值**与缺省跑逐条相同（跳进上界⛔ 不筛点、⛔ 不改时值）；
/// * 收紧后的实测最大跳进 ≤ 请求上界（正例真的被消费）；
/// * 至少一条旋律音符的音高被改变（否则正例是空判据）；
/// * 和声声部与鼓与缺省跑**逐字节**相同（上界⛔ 溢出到别的声部）；
/// * 力度仍然只取阶梯的两档（⚠ 不逐条对齐，见文件头）。
#[test]
fn a_tight_leap_moves_only_the_melody_pitches() {
    let base = plan(Given::theory());
    let base_table = voices(&base);
    let base_melody = &base_table["Melody"];
    let base_leap = max_leap(base_melody);

    for bound in [NARROW, STEPWISE] {
        let shifted = plan(Given::theory().leap(Some(bound)));
        let shifted_table = voices(&shifted);
        assert_eq!(
            base_table.keys().collect::<Vec<_>>(),
            shifted_table.keys().collect::<Vec<_>>(),
            "上界 {bound}：声部集合必须不变"
        );
        for part in preset_parts(STYLE).expect("夹具必须在册") {
            assert_eq!(
                base_table[*part], shifted_table[*part],
                "上界 {bound}：和声声部 `{part}` 必须与缺省跑逐条相同（跳进上界只作用于旋律）"
            );
        }
        assert_eq!(
            base_table["Drums"], shifted_table["Drums"],
            "上界 {bound}：鼓声部必须与缺省跑逐条相同（鼓的音高是 GM 键）"
        );
        let shifted_melody = &shifted_table["Melody"];
        assert_eq!(
            base_melody.keys().collect::<Vec<_>>(),
            shifted_melody.keys().collect::<Vec<_>>(),
            "上界 {bound}：旋律的音符 id 集合必须⛔ 不受跳进上界影响（不筛点）"
        );
        let mut moved_ids: Vec<&String> = Vec::new();
        for (id, (start, duration, pitch, velocity)) in base_melody {
            let (shift_start, shift_duration, shift_pitch, shift_velocity) = shifted_melody[id];
            assert_eq!(
                (start, duration),
                (&shift_start, &shift_duration),
                "上界 {bound}：旋律音符 `{id}` 的起点与时值不得因跳进上界改变"
            );
            assert!(
                LADDER.contains(velocity),
                "上界 {bound}：缺省跑的旋律音符 `{id}` 力度 {velocity} 本就该取阶梯两档 {LADDER:?}"
            );
            assert!(
                LADDER.contains(&shift_velocity),
                "上界 {bound}：旋律音符 `{id}` 的力度 {shift_velocity} 必须仍取阶梯两档 {LADDER:?}"
            );
            if *pitch != shift_pitch {
                moved_ids.push(id);
            }
        }
        let observed = max_leap(shifted_melody);
        assert!(
            observed <= bound,
            "上界 {bound}：实测最大跳进 {observed} 必须 ≤ 请求上界（theory 明文⛔ 静默放宽）"
        );
        assert!(
            !moved_ids.is_empty(),
            "上界 {bound}：必须真的改变至少一条旋律音符的音高（否则正例是空判据）。缺省实测跳进 = {base_leap}"
        );
    }

    // 出厂上界必须**不是**空语义：缺省跑的实测最大跳进必须大于 `STEPWISE`，
    // 否则"收紧到 2"就没有可观察的变化可言（本条同时是 D1 的"这把尺子真的在量"）。
    assert!(
        base_leap > STEPWISE,
        "夹具的缺省旋律必须真的出现 > {STEPWISE} 的跳进，否则本文件的上界判据全是空判据: 实测 {base_leap}"
    );
}

/// ⭐ 判据 3（D1/D2 的形状面）：越界（`128` / 超出 `u8` 宽度）⇒ 点名是哪个键，
/// 载荷带 `value` + `range`，⛔ `reason` 键。`127` 与 `0` 是**边界内的两个端点** ⇒ 必须放行。
#[test]
fn out_of_range_is_rejected_naming_the_key() {
    for raw in [128u64, 1_000, u64::from(u8::MAX) + 1] {
        let fault = try_plan(Given::theory().leap(Some(raw)))
            .expect_err("越界不得静默夹紧（`ADR-0036` D3 的同一条口径）");
        assert_shape_reject(&fault, &format!("`melodyMaxLeap` 越界 ({raw})"));
        let payload = data(&fault);
        assert_eq!(
            payload["field"],
            json!("melodyMaxLeap"),
            "载荷必须点名是哪个键: {payload}"
        );
        assert_eq!(
            payload["value"],
            json!(raw),
            "载荷必须回显被拒的值: {payload}"
        );
        assert_eq!(
            payload["range"],
            json!([0, 127]),
            "载荷必须给出界（`0..=MIDI_PITCH_MAX`）: {payload}"
        );
    }
    // 两个端点都在界内 ⇒ 必须放行（`127` = "实际不设限"是语义，⛔ 越界）。
    for raw in [0u64, 127] {
        let given = Given::theory().leap(Some(raw));
        try_plan(given).unwrap_or_else(|fault| {
            panic!(
                "`melodyMaxLeap` = {raw} 在 0..=127 内，必须放行（端点⛔ 当越界处理）: {fault:?}"
            )
        });
    }
}

/// ⭐ 判据 4（D3 的消费面）：本键⛔ 脱离 `materialSource:"theory"`，也⛔ 脱离
/// `includeMelody: true` —— 与音域两键共用同一条校验（消费点是同一个）。
#[test]
fn leap_key_requires_theory_source_and_a_melody_part() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .leap(Some(STEPWISE)),
        )
        .expect_err("`melodyMaxLeap` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`melodyMaxLeap` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    // 有 theory 但没有旋律声部 ⇒ 同样必须拒（键缺省 `includeMelody:false`）。
    let fault = try_plan(Given::theory().leap(Some(STEPWISE)).melody(false))
        .expect_err("`melodyMaxLeap` 不得在没有旋律声部时静默通过");
    assert_shape_reject(&fault, "`melodyMaxLeap` 脱离 includeMelody");
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeMelody")),
        "载荷必须指出要补 includeMelody: {}",
        data(&fault)
    );
}

/// ⭐ 判据 5（D1/D2 的极端合法面）：`0` = 只允许同音反复 ⇒ **合法**，整条旋律被压成
/// 一个音高，而 onset 数与缺省跑相同（⛔ 报错、⛔ 产出空旋律、⛔ 静默放宽上界）。
#[test]
fn a_zero_leap_is_legal_and_repeats_one_pitch() {
    let base = plan(Given::theory());
    let base_melody = &voices(&base)["Melody"];
    let repeated = plan(Given::theory().leap(Some(0)));
    let repeated_melody = &voices(&repeated)["Melody"];
    assert_eq!(
        base_melody.keys().collect::<Vec<_>>(),
        repeated_melody.keys().collect::<Vec<_>>(),
        "`melodyMaxLeap: 0` 必须⛔ 改变音符 id 集合（不筛点）"
    );
    assert_eq!(
        max_leap(repeated_melody),
        0,
        "`melodyMaxLeap: 0` 必须把整条旋律压成同音反复"
    );
    assert_eq!(
        pitches(repeated_melody).len(),
        1,
        "`melodyMaxLeap: 0` 后只应剩一个音高: {:?}",
        pitches(repeated_melody)
    );
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON），且契约的 `default`
/// 与 theory 的出厂上界是**同一组数字**；类型必须是 integer（工具面的"非整数 / 负数"
/// 由这道类型闸承担 —— 见文件头）。
///
/// ⚠ 本票同时翻转了 `ADR-0038` 判据 6 里的反向钉（那条钉的是"`melodyMaxLeap` ⛔ 存在"）：
/// 键现在合法存在，反向钉已改成"必须存在"，形状由本条逐字段核。
#[test]
fn leap_bound_is_declared_on_both_param_surfaces() {
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
    let param = spec
        .param("melodyMaxLeap")
        .expect("参数表必须声明 `melodyMaxLeap`");
    assert_eq!(param.json_type, "integer", "`melodyMaxLeap` 必须是整数参数");
    assert!(!param.required, "缺省 = 出厂上界 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0039") && param.doc.contains("半音距离"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );
    let property = &declared["melodyMaxLeap"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `melodyMaxLeap` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("integer"));
    assert_eq!(
        property["minimum"],
        json!(0),
        "下界必须是 0（同音反复合法）"
    );
    assert_eq!(
        property["maximum"],
        json!(127),
        "上界必须与 `MIDI_PITCH_MAX` 同一组数字（半音距离⛔ 可能超过音高全域宽度）: {property}"
    );
    assert_eq!(
        property["default"],
        json!(MELODY_MAX_LEAP),
        "缺省必须与 theory 的出厂上界同源（⛔ 两份数字）: {property}"
    );
}
