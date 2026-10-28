//! `ADR-0041`（缺口 `G8i`：鼓的摇摆比例是入参）：`yeban_propose_section` 的
//! `swingPermille` —— 把 `GenreRule::drum_pattern` 里**写死为流派登记值**的那个实参
//! （`swung_styled_drum_pattern` 的 `permille`）开放成可给的入参，缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **千分比**（`500..=1000`，`500` = 平直 = 不推任何一个 onset），⛔ 不是百分数、
//! ⛔ 不是 tick 数、⛔ 不是"每小节推多快"。界 = theory 公开的两个常量
//! （`SWING_PERMILLE_STRAIGHT` / `SWING_PERMILLE_MAX`），本层⛔ 另立一份数字（`ADR-0041` D2）。
//!
//! ⚠ **本键与前面四票最大的一处不同：缺省不是一个数字**（读数：现读 `GENRES` 182 条里
//! 148 条不登记摇摆、34 条登记 `540..=660`，⛔ 一条登记 `500`）⇒ 键缺席的语义是
//! "**不覆盖**该流派登记的值"，契约里因此⛔ 写 `default`（D5）。夹具 `synthwave` 属"不登记"
//! 那一支，实测它的登记路径与显式 `Some(500)` 逐条相同（88 条 hit 的 `(音色, tick, 重音)` 全等），
//! 所以判据 1 才能在**本层**写"缺席 ≡ 500"；那是一条**实测等价**，⛔ 定义。
//!
//! ⚠ **消费点只有鼓轨**：旋律的 onset 网格与和声⛔ 读这个键（`ADR-0041` D3，旋律网格的摇摆
//! 登记为非范围 `N1`）⇒ 判据 2 把"只改鼓"钉成"别的声部逐字节相同"，判据 4 反钉"没有旋律声部
//! 时必须放行"。耦合口径与 `drumDensity` 同一支（要鼓轨在场），与和声键那一支（只要求材料源）
//! **不同** ⇒ 三个口径⛔ 折成一个表达式。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与 `ParamSpec::accepts("integer")`），
//! 纯领域层量不到 ⇒ 写成判据 6 里"`json_type` 必须是 integer"（与 `ADR-0036`..`ADR-0040` 同一条分工）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::{SWING_PERMILLE_MAX, SWING_PERMILLE_STRAIGHT};

/// 夹具的小节数（与力度/音域/跳进三票同一份夹具）。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4、不登记摇摆 ⇒ 缺省 = 平直。
const STYLE: &str = "synthwave";
/// 夹具的调。
const KEY: &str = "C minor";
/// 夹具的进行。
const PROGRESSION: &str = "i-VI-III-VII";
/// 平直档（= theory 的 `SWING_PERMILLE_STRAIGHT`，⛔ 在本文件抄 `500`）。
const STRAIGHT: u64 = SWING_PERMILLE_STRAIGHT as u64;
/// 一档中等摇摆（千分比）：实测与平直的位移和 = 1216 tick（4 小节 16 格位）。
const MEDIUM: u64 = 580;
/// 一档更紧的摇摆（千分比）：位移必须严格大于 [`MEDIUM`]。
const WIDE: u64 = 700;
/// 上界端点（千分比）：100% swing，位移必须最大且仍⛔ 被本层拒。
const FULL: u64 = SWING_PERMILLE_MAX as u64;
/// 缺省阶梯的两档（本文件的夹具不给力度键）。
const LADDER: [u8; 2] = [96, 112];

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律 + 夹具进行，本键不给）。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    include_drums: bool,
    include_melody: bool,
    swing_permille: Option<u64>,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_drums: true,
            include_melody: true,
            swing_permille: None,
        }
    }

    fn swing(mut self, value: Option<u64>) -> Self {
        self.swing_permille = value;
        self
    }

    fn material_source(mut self, value: Option<&'static str>) -> Self {
        self.material_source = value;
        self
    }

    fn drums(mut self, value: bool) -> Self {
        self.include_drums = value;
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
            progression: Some(PROGRESSION),
            include_drums: Some(given.include_drums),
            include_melody: Some(given.include_melody),
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: None,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: None,
            harmony_max_voice_jump: None,
            swing_permille: given.swing_permille,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（swingPermille={:?}）必须成功: {fault:?}",
            given.swing_permille
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

/// 两次运行之间**鼓轨**同一 id 的起点位移之和（单位 = tick）。
///
/// ⚠ 只量鼓轨：本键的消费点就是它（`ADR-0041` D3），别的声部的位移⛔ 是这条链上的信号。
fn drum_shift(base: &BTreeMap<String, Note>, other: &BTreeMap<String, Note>) -> u64 {
    other
        .iter()
        .map(|(id, note)| match base.get(id) {
            Some(previous) => previous.0.abs_diff(note.0),
            None => panic!("鼓音符 id 集合必须不随摇摆比例改变（本键⛔ 不筛点）: {id}"),
        })
        .sum()
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0041` D7 继承 `ADR-0038` D4 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
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

/// ⭐ 判据 1（`ADR-0041` D1/D2/D5）：**键缺席 ≡ 显式给平直档** ⇒ 整个规划逐字节相同
/// （`SectionPlan: PartialEq`）。夹具 `synthwave` 不登记摇摆（读数：`GENRES` 182 条里 148 条不登记），
/// 实测它的登记路径与 `Some(500)` 逐条相同 ⇒ 这条等价是**实测**而⛔ 定义（文件头）。
/// 顺带钉住夹具的形状：鼓轨多条、和声声部在册、鼓的力度只取阶梯两档。
#[test]
fn absent_swing_is_byte_identical_to_the_straight_permille() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().swing(Some(STRAIGHT)));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"swingPermille\": {STRAIGHT}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let drums = &table["Drums"];
    // 夹具必须真的产出一条有内容的鼓轨（否则判据 2/5 都是空判据）。
    assert!(
        drums.len() > 1,
        "夹具的鼓轨必须有多于一音符: {}",
        drums.len()
    );
    for (_, _, _, velocity) in drums.values() {
        assert!(
            LADDER.contains(velocity),
            "缺省鼓音符的力度必须取阶梯的两档 {LADDER:?}，实际 {velocity}"
        );
    }
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert!(
            table[*part].len() > 1,
            "夹具的和声声部 `{part}` 必须有多于一层和弦"
        );
    }
}

/// ⭐ 判据 2（D1 的映射面 + "只作用于鼓"）：给 `WIDE` ⇒
/// * 鼓轨的**音符 id 集合、音高、时值、力度**逐条与缺省跑相同（本键⛔ 不筛点、⛔ 不改音色、
///   ⛔ 不改时值、⛔ 不改力度）；
/// * 起点**至少一条被推移**（防空判据：读数说 `580` 档真的推动 32 条反拍）；
/// * 和声各声部与旋律与缺省跑**逐字节**相同（摇摆⛔ 溢出到别的声部 —— 旋律网格的摇摆
///   是 `ADR-0041` 的非范围 `N1`）。
#[test]
fn a_tighter_swing_moves_only_the_drum_starts() {
    let base = plan(Given::theory());
    let base_table = voices(&base);
    let shifted = plan(Given::theory().swing(Some(WIDE)));
    let shifted_table = voices(&shifted);

    assert_eq!(
        base_table.keys().collect::<Vec<_>>(),
        shifted_table.keys().collect::<Vec<_>>(),
        "摇摆 {WIDE}：声部集合必须不变"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert_eq!(
            base_table[*part], shifted_table[*part],
            "摇摆 {WIDE}：和声声部 `{part}` 必须与缺省跑逐条相同（本键只作用于鼓轨）"
        );
    }
    assert_eq!(
        base_table["Melody"], shifted_table["Melody"],
        "摇摆 {WIDE}：旋律声部必须与缺省跑逐条相同（旋律网格的摇摆是本票非范围 `N1`）"
    );

    let base_drums = &base_table["Drums"];
    let shifted_drums = &shifted_table["Drums"];
    assert_eq!(
        base_drums.keys().collect::<Vec<_>>(),
        shifted_drums.keys().collect::<Vec<_>>(),
        "摇摆 {WIDE}：鼓音符的 id 集合必须⛔ 受影响（本键⛔ 不筛点）"
    );
    let mut moved = 0usize;
    for (id, (start, duration, pitch, velocity)) in base_drums {
        let (shift_start, shift_duration, shift_pitch, shift_velocity) = shifted_drums[id];
        assert_eq!(
            (duration, pitch, velocity),
            (&shift_duration, &shift_pitch, &shift_velocity),
            "摇摆 {WIDE}：鼓音符 `{id}` 的时值/音高/力度不得因摇摆改变（音高 = GM 键，力度 = 阶梯档位）"
        );
        if *start != shift_start {
            moved += 1;
        }
    }
    assert!(
        moved > 0,
        "摇摆 {WIDE} 必须真的推移至少一条鼓音符的起点（否则正例是空判据）"
    );
}

/// ⭐ 判据 3（D2 的形状面）：越界 ⇒ 点名是哪个键、载荷带 `value` + `range:[500,1000]`、
/// ⛔ `reason` 键；两个端点（`500` 与 `1000`）必须放行。
/// ⚠ `0` / `499` / `1001` 都**必须**是本层的参数拒绝（载荷带 `field`）—— theory 侧对
/// `499`/`1001` 也给 `SwingOutOfRange`，但那一支在本层就够不着（D4：界只判一次）。
#[test]
fn out_of_range_is_rejected_naming_the_key() {
    for raw in [0u64, 499, 1_001, u64::from(u16::MAX) + 1, 100_000] {
        let fault = try_plan(Given::theory().swing(Some(raw)))
            .expect_err("越界不得静默夹紧（`ADR-0036` D3 的同一条口径）");
        assert_shape_reject(&fault, &format!("`swingPermille` 越界 ({raw})"));
        let payload = data(&fault);
        assert_eq!(
            payload["field"],
            json!("swingPermille"),
            "载荷必须点名是哪个键: {payload}"
        );
        assert_eq!(
            payload["value"],
            json!(raw),
            "载荷必须回显被拒的值: {payload}"
        );
        assert_eq!(
            payload["range"],
            json!([STRAIGHT, FULL]),
            "载荷必须给出 theory 公开的那对界: {payload}"
        );
    }
    for bound in [STRAIGHT, FULL] {
        try_plan(Given::theory().swing(Some(bound))).unwrap_or_else(|fault| {
            panic!("`swingPermille` = {bound} 在界内，必须放行: {fault:?}")
        });
    }
}

/// ⭐ 判据 4（D3 的消费面，**两向都钉**）：本键要求材料源 = theory **且**鼓轨在场；
/// 反钉"⛔ 把旋律那条更严的要求扩散过来" —— `includeMelody:false` 时给本键必须放行。
#[test]
fn the_swing_key_requires_the_theory_source_and_the_drum_part() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .swing(Some(MEDIUM)),
        )
        .expect_err("`swingPermille` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`swingPermille` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    // 鼓轨不在场 ⇒ 本键无人消费，必须响亮拒绝而不是静默忽略。
    let fault = try_plan(Given::theory().drums(false).swing(Some(MEDIUM)))
        .expect_err("`swingPermille` 不得在没有鼓轨时静默失效");
    assert_shape_reject(&fault, "`swingPermille` 脱离 includeDrums");
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeDrums")),
        "载荷必须指出要补 includeDrums: {}",
        data(&fault)
    );
    // 反钉：没有旋律声部时必须放行（摇摆只被鼓轨消费）。
    try_plan(Given::theory().melody(false).swing(Some(MEDIUM))).unwrap_or_else(|fault| {
        panic!("`swingPermille` ⛔ 要求旋律声部在场（旋律网格的摇摆是本票非范围 `N1`）: {fault:?}")
    });
}

/// ⭐ 判据 5（"给了必须被消费"的强度读数）：起点位移随比例**单调不减**，且
/// * 平直档的位移恰为 `0`（= 判据 1 的逐字节相同，两处读数必须互相印证）；
/// * `MEDIUM < WIDE < FULL` 三档的位移和严格递增（单位 = tick）。
///
/// 这条是"⛔ 给了不生效"那类缺陷的**量化**防线（口径来自 `ADR-0037` D5）：判据 2 只说
/// "至少一条变了"，这里说"变的方向与请求的比例一致" —— 后者才能挡住"接错实参"（例如把
/// 千分比当成 tick 数或当成百分数传下去：那会位移和与档位不成单调关系）。
#[test]
fn the_displacement_grows_monotonically_with_the_permille() {
    let base = voices(&plan(Given::theory()));
    let mut previous = 0u64;
    for bound in [STRAIGHT, MEDIUM, WIDE, FULL] {
        let shifted = voices(&plan(Given::theory().swing(Some(bound))));
        let shift = drum_shift(&base["Drums"], &shifted["Drums"]);
        if bound == STRAIGHT {
            assert_eq!(
                shift, 0,
                "平直档（{STRAIGHT}）的位移必须恰为 0 tick（与判据 1 的逐字节相同互证）"
            );
        } else {
            assert!(
                shift > previous,
                "摇摆 {bound} 的位移和 {shift} 必须严格大于上一档 {previous}（单调性 = 这个比例真的被消费）"
            );
        }
        previous = shift;
    }
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON），且界与 theory 公开的那对
/// 常量是**同一组数字**；契约⛔ 登记 `default`（`ADR-0041` D5：缺省随流派移动 ⇒ 一个固定
/// 数字会把 148 条与 34 条说成同一个）。类型必须是 integer（工具面的"非整数 / 负数"由这道
/// 类型闸承担 —— 见文件头）。
#[test]
fn swing_permille_is_declared_on_both_param_surfaces_without_a_default() {
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
        .param("swingPermille")
        .expect("参数表必须声明 `swingPermille`");
    assert_eq!(param.json_type, "integer", "`swingPermille` 必须是整数参数");
    assert!(!param.required, "缺省 = 该流派登记值 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0041") && param.doc.contains("千分比"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );
    let property = &declared["swingPermille"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `swingPermille` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("integer"));
    assert_eq!(
        property["minimum"],
        json!(STRAIGHT),
        "下界必须与 theory 的 `SWING_PERMILLE_STRAIGHT` 同一组数字（⛔ 两份数字）: {property}"
    );
    assert_eq!(
        property["maximum"],
        json!(FULL),
        "上界必须与 theory 的 `SWING_PERMILLE_MAX` 同一组数字: {property}"
    );
    assert!(
        property.get("default").is_none(),
        "本键⛔ 写固定 `default`：现读 `GENRES` 里 148 条不登记、34 条登记 540..=660，\
         缺省语义是\"不覆盖该流派登记值\"（`ADR-0041` D5）: {property}"
    );
}
