//! `ADR-0037`（审计缺口 `G8` 的力度轴）：`yeban_propose_section` 的
//! `velocityMin` / `velocityMax` —— 既有**两级**力度阶梯（重音／和弦音 vs 其余）
//! 从常量换成可给的两个入参，缺省 `96` / `112` ⇒ 逐字节不变。
//!
//! 单位口径 = **MIDI velocity**（`0..=127`），⛔ 不是 dB、⛔ 不是"每条音符一个数"。
//! 两档的**归属**由 theory 的登记决定（鼓 `hit.accent`、旋律 `chord_tone`、和声无
//! tier 概念 ⇒ 全部下档），本票⛔ 不重新发明哪一拍算重音（那是 `ADR-0028` 的口径）。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与
//! `ParamSpec::accepts("integer")`），纯领域层量不到 ⇒ 本文件把它写成
//! [`velocity_is_declared_on_both_param_surfaces`] 里"`json_type` 必须是 integer"，
//! 领域层能量的形状（越界 / 倒序 / 缺省兼容）写成下面几条
//! （与 `ADR-0036` 判据 4 的同一条分工）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::cells_per_bar;
use yeban_theory::genre::GenreLibrary;

/// 夹具的小节数（鼓组型的独立重算要用同一个数）。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4（⇒ 缺省网格 16）**且**登记了重音击点（`ADR-0028` 的
/// `sparse` 判据实测过它）⇒ 阶梯的两档在这条流派上**都**有音符。
const STYLE: &str = "synthwave";

/// 缺省阶梯的两个值（`ADR-0017` D1 的常量，本票起为**缺省值**）。
const DEFAULT_MIN: u8 = 96;
const DEFAULT_MAX: u8 = 112;

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
///
/// ⚠ 键是 id 而不是"排序后的下标"：id 集合相等 ⇒ "力度⛔ 参与 id 派生"这条判据
/// 才真的被数到（下标对齐会把顺序变化糊掉）。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项。
///
/// ⛔ 每个判据手抄一遍 `PlanOptions` 的十个字段：那张表就是本文件的"缺省 + 一处改动"。
#[derive(Clone, Copy)]
struct Given<'a> {
    material_source: Option<&'a str>,
    include_drums: bool,
    include_melody: bool,
    velocity_min: Option<u64>,
    velocity_max: Option<u64>,
}

impl<'a> Given<'a> {
    /// `materialSource: "theory"` + 鼓 + 旋律，阶梯两键都不给。
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_drums: true,
            include_melody: true,
            velocity_min: None,
            velocity_max: None,
        }
    }

    fn min(mut self, value: Option<u64>) -> Self {
        self.velocity_min = value;
        self
    }

    fn max(mut self, value: Option<u64>) -> Self {
        self.velocity_max = value;
        self
    }

    fn material_source(mut self, value: Option<&'a str>) -> Self {
        self.material_source = value;
        self
    }

    /// 只留和声一条路径（`ADR-0037` D4 的正例：和声声部**恒**消费下档）。
    fn consumers(mut self, drums: bool, melody: bool) -> Self {
        self.include_drums = drums;
        self.include_melody = melody;
        self
    }
}

fn try_plan(given: Given<'_>) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        STYLE,
        BARS,
        Some("C minor"),
        PlanOptions {
            material_source: given.material_source,
            progression: None,
            include_drums: Some(given.include_drums),
            include_melody: Some(given.include_melody),
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: given.velocity_min,
            velocity_max: given.velocity_max,
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: None,
        },
    )
}

fn plan(given: Given<'_>) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（min={:?} max={:?}）必须成功: {fault:?}",
            given.velocity_min, given.velocity_max
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

/// 某声部的力度多重集（去重后升序）。
fn rungs(table: &BTreeMap<String, Note>) -> Vec<u8> {
    let mut values: Vec<u8> = table.values().map(|note| note.3).collect();
    values.sort_unstable();
    values.dedup();
    values
}

/// 该夹具流派缺省网格的格位数（⛔ 手抄 16：从 `cells_per_bar(拍号)` 现读）。
fn grid_cells() -> u32 {
    let rule = GenreLibrary::get(STYLE).expect("夹具流派必须在册");
    u32::try_from(cells_per_bar(rule.meter_value()).expect("夹具拍号必须合法"))
        .expect("格位数进 u32")
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0037` D3：故障词由既有的形状词承担，本票⛔ 新增 `reason` ⇒
/// `fault_vocabulary` 的词表与规范面摘要不变）。
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

/// ⭐ 判据 1（`ADR-0037` D2）：**两键都不给 ≡ 给 `96`/`112`**，整个规划逐字节相同
/// （`SectionPlan: PartialEq`）；且缺省输出的力度集合恰为既有的两档，
/// 和声声部全是下档。
#[test]
fn absent_ladder_is_byte_identical_to_the_published_two_rungs() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().min(Some(96)).max(Some(112)));
    assert_eq!(
        absent, explicit,
        "两个键缺席必须逐字节等于给 `{{\"velocityMin\": 96, \"velocityMax\": 112}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let mut seen: Vec<u8> = table
        .values()
        .flat_map(|notes| notes.values().map(|note| note.3))
        .collect();
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(
        seen,
        vec![DEFAULT_MIN, DEFAULT_MAX],
        "缺省输出的力度集合必须恰为既有的两档 `{DEFAULT_MIN}`/`{DEFAULT_MAX}`: {seen:?}"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        let notes = &table[*part];
        assert!(
            notes.values().all(|note| note.3 == DEFAULT_MIN),
            "和声声部 `{part}` 的力度必须全是下档 {DEFAULT_MIN}: {:?}",
            rungs(notes)
        );
    }
}

/// ⭐ 判据 2（D1 的映射面）：给 `{40, 100}` ⇒
/// * **音符 id 集合、起点、时值、音高**与缺省跑逐条相同（力度⛔ 参与 id 派生 ——
///   种子只含 section/声部/序号，本判据把这条钉住）；
/// * 每条音符按**缺省那一档**换到新档（112 → 100、96 → 40）；
/// * 鼓的 100 档**逐击**等于 theory 登记为重音的子集（期望值从 `drum_pattern`
///   独立重算，⛔ 读本模块中间量）；
/// * 旋律两档都在场（否则本判据退化成"只测了下档"）。
#[test]
fn the_two_rungs_land_on_exactly_the_registered_accent_tiers() {
    let base = plan(Given::theory());
    let shifted = plan(Given::theory().min(Some(40)).max(Some(100)));
    let base_table = voices(&base);
    let shifted_table = voices(&shifted);
    assert_eq!(
        base_table.keys().collect::<Vec<_>>(),
        shifted_table.keys().collect::<Vec<_>>(),
        "两跑的声部集合必须相同"
    );
    for (part, base_notes) in &base_table {
        let shifted_notes = &shifted_table[part];
        assert_eq!(
            base_notes.keys().collect::<Vec<_>>(),
            shifted_notes.keys().collect::<Vec<_>>(),
            "声部 `{part}`：音符 id 集合必须⛔ 不受力度影响"
        );
        for (id, note) in base_notes {
            let (start, duration, pitch, velocity) = *note;
            let (shift_start, shift_duration, shift_pitch, shift_velocity) = shifted_notes[id];
            assert_eq!(
                (start, duration, pitch),
                (shift_start, shift_duration, shift_pitch),
                "声部 `{part}` 音符 `{id}`：位置与时值不得因力度改变"
            );
            let expected = match velocity {
                DEFAULT_MIN => 40,
                DEFAULT_MAX => 100,
                other => panic!("声部 `{part}` 出现了阶梯外的缺省力度 {other}"),
            };
            assert_eq!(
                shift_velocity, expected,
                "声部 `{part}` 音符 `{id}`：缺省 {velocity} 档必须换到新 {expected} 档"
            );
        }
    }

    // 和声声部：整条落在下档。
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert_eq!(
            rungs(&shifted_table[*part]),
            vec![40],
            "和声声部 `{part}` 必须整条落在 `velocityMin` = 40"
        );
    }

    // 鼓：100 档**逐击** = theory 的重音击点集合；40 档 = 其余击点。
    let pattern = GenreLibrary::get(STYLE)
        .expect("夹具流派必须在册")
        .drum_pattern(u32::try_from(BARS).expect("BARS 进 u32"), grid_cells())
        .expect("鼓组型")
        .expect("夹具拍号合法 ⇒ 有鼓组型");
    let hits = pattern.hits();
    let accented: Vec<(u64, u8)> = hits
        .iter()
        .filter(|hit| hit.accent)
        .map(|hit| (hit.tick, hit.voice.gm_key()))
        .collect();
    let plain: Vec<(u64, u8)> = hits
        .iter()
        .filter(|hit| !hit.accent)
        .map(|hit| (hit.tick, hit.voice.gm_key()))
        .collect();
    assert!(
        !accented.is_empty() && !plain.is_empty(),
        "夹具流派必须同时有重音与非重音击点（否则两档判据是空的）: 重音 {} / 其余 {}",
        accented.len(),
        plain.len()
    );
    assert_eq!(
        hits.len(),
        shifted_table["Drums"].len(),
        "鼓的音符数必须等于鼓组型的击点数（normal 档不筛点）"
    );
    let mut actual_up: Vec<(u64, u8)> = shifted_table["Drums"]
        .values()
        .filter(|note| note.3 == 100)
        .map(|note| (note.0, note.2))
        .collect();
    let mut expected_up = accented;
    actual_up.sort_unstable();
    expected_up.sort_unstable();
    assert_eq!(
        actual_up, expected_up,
        "鼓的上档必须逐击等于 theory 登记的重音击点（起点 tick + GM 键）"
    );
    assert_eq!(
        rungs(&shifted_table["Drums"]),
        vec![40, 100],
        "鼓必须两档都有音符"
    );

    // 旋律：同样两档都在场（`chord_tone` 有真也有假）。
    assert_eq!(
        rungs(&shifted_table["Melody"]),
        vec![40, 100],
        "旋律必须两档都有音符（否则本票在旋律轴上是空判据）"
    );
}

/// ⭐ 判据 3（D3 的形状面）：越界（`128` / 超出 `u8` 宽度）⇒ 点名是哪个键；
/// 倒序 ⇒ `min > max` 拒绝并给出两档；`min == max` 合法（全曲单档）。
#[test]
fn out_of_range_and_inverted_are_rejected_naming_the_key() {
    for key in ["velocityMin", "velocityMax"] {
        for raw in [128u64, 1_000, u64::from(u8::MAX) + 1] {
            let given = if key == "velocityMin" {
                Given::theory().min(Some(raw))
            } else {
                Given::theory().max(Some(raw))
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
                "载荷必须给出 MIDI velocity 的界: {payload}"
            );
        }
    }
    // 倒序：`min > max` ⇒ 反重音不属于力度范围的语义（D3 / N2）。
    let fault = try_plan(Given::theory().min(Some(100)).max(Some(40)))
        .expect_err("反重音（min > max）必须被拒绝，⛔ 当成合法输入");
    assert_shape_reject(&fault, "倒序阶梯");
    let payload = data(&fault);
    assert_eq!(payload["field"], json!("velocityMin"), "载荷: {payload}");
    assert_eq!(payload["velocityMin"], json!(100), "载荷: {payload}");
    assert_eq!(payload["velocityMax"], json!(40), "载荷: {payload}");
    assert!(
        payload["hint"]
            .as_str()
            .is_some_and(|text| text.contains("velocityMin")),
        "载荷必须给出可执行的修正方向: {payload}"
    );
    // `min == max` 合法：全曲单档。
    let single = plan(Given::theory().min(Some(70)).max(Some(70)));
    let table = voices(&single);
    let all: Vec<u8> = table
        .values()
        .flat_map(|notes| notes.values().map(|note| note.3))
        .collect();
    assert!(
        !all.is_empty() && all.iter().all(|value| *value == 70),
        "`min == max` 必须合法并把全部音符压到那一档: {:?}",
        all.len()
    );
}

/// ⭐ 判据 4（D4 的作用域）：给了必须被消费 —— 非 theory 的材料路径给任一键即拒；
/// 正例：**只有和声**（无鼓无旋律）也必须成功并消费下档（⛔ 沿用 `onsetsPerBar`
/// 那条更严的"必须有鼓或旋律"，因为和声声部恒消费下档）。
#[test]
fn velocity_keys_require_theory_source_and_are_consumed_by_harmony() {
    for material_source in [None, Some("project")] {
        for which in ["velocityMin", "velocityMax"] {
            let given = if which == "velocityMin" {
                Given::theory()
                    .material_source(material_source)
                    .min(Some(50))
            } else {
                Given::theory()
                    .material_source(material_source)
                    .max(Some(120))
            };
            let fault = try_plan(given.consumers(false, false))
                .expect_err("`{which}` 不得脱离 materialSource:\"theory\" 单独给");
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
    // 正例：只留和声 —— 两档里只有下档有消费点，仍必须成功。
    let harmony_only = plan(Given::theory().min(Some(33)).consumers(false, false));
    let table = voices(&harmony_only);
    assert!(
        !table.contains_key("Drums") && !table.contains_key("Melody"),
        "夹具必须真的没有鼓与旋律: {:?}",
        table.keys().collect::<Vec<_>>()
    );
    assert!(
        !table.is_empty(),
        "和声声部必须真的生成出来（否则正例是空判据）"
    );
    for (part, notes) in &table {
        assert_eq!(
            rungs(notes),
            vec![33],
            "只有和声时 `velocityMin` 必须仍然生效（和声恒消费下档）: `{part}`"
        );
    }
}

/// ⭐ 判据 5：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON），且契约的
/// `default` 与领域层的缺省两档是**同一组数字**；类型必须是 integer
/// （工具面的"非整数 / 负数"由这道类型闸承担 —— 见文件头）。
#[test]
fn velocity_is_declared_on_both_param_surfaces() {
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
    for (name, default) in [("velocityMin", DEFAULT_MIN), ("velocityMax", DEFAULT_MAX)] {
        let param = spec
            .param(name)
            .unwrap_or_else(|| panic!("参数表必须声明 `{name}`"));
        assert_eq!(param.json_type, "integer", "`{name}` 必须是整数参数");
        assert!(!param.required, "缺省 = 既有两档 ⇒ 必须可选");
        assert!(
            param.doc.contains("ADR-0037") && param.doc.contains("velocity"),
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
            "`{name}` 的上界必须与领域层的界同一组数字: {property}"
        );
        assert_eq!(
            property["default"],
            json!(default),
            "`{name}` 的缺省必须与领域层常量同源（⛔ 两份数字）: {property}"
        );
    }
}
