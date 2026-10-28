//! `ADR-0043`（缺口 `G8k`：军鼓落点是入参）：`yeban_propose_section` 的 `drumBackbeat` ——
//! 把 `GenreRule::drum_pattern` 里**写死为派生值**的那个实参（`swung_styled_drum_pattern` 的
//! `backbeat`）开放成可给的入参，缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **拍序号**（第 0 基；语义 = 军鼓打在拍序号能被它整除的那些**拍的起点**上，
//! `1` = 每一拍都打），⛔ 不是小节号、⛔ 不是 tick、⛔ 不是"每小节打几下"。
//!
//! ⚠ **本键的界与 `swingPermille` 那票是两支**（`ADR-0043` D1/D2）：上界⛔ 是常量，而是
//! **该拍号的拍数** —— 本层调 theory 的 `felt_beats_per_bar(meter)` 现读（4/4 ⇒ `1..=4`、
//! 3/4 ⇒ `1..=3`）。缺省值也**随拍号移动**（`default_backbeat`：偶数拍取中点、奇数拍取中间那拍
//! ⇒ 4/4→`2`、3/4→`2`、2/4 与 6/8→`1`、5/4→`3`）⇒ 契约里因此⛔ 写 `default`、⛔ 写 `maximum`。
//! 判据 5 就是钉这一支：同一个数在 4/4 上放行、在 3/4 上被拒，且 `range` 里的上界不同。
//!
//! ⚠ **本键与摇摆那根轴**（`ADR-0041`）**相反的一面**：摇摆恒 88 条、只推 tick；本键会
//! **加／筛击点**（实测 4/4 夹具 `96`(`1`) ↔ `88`(`2`=缺省) ↔ `84`(`4`) 条，军鼓 `16` ↔ `8` ↔ `4` 条）。
//! 于是 `G8i` 判据 2 那句"鼓轨 id 集合逐条不变"在此⛔ 能用 —— 击点的 id 由**原始 hit 序号**派生
//! （`drum_contents` 的 `theory-drum:{section}:{hit_index}:{voice}`），条数一变序号就移。
//! 因此本文件比的是**形状多重集**（非军鼓击点的 `(起点, 时值, 音色, 力度)` 排序后全等）与
//! **落点拍集合**（读数 3：军鼓条数⛔ 单调 ⇒ 5/4 上 `3` 与 `4` 都得 8 条而落点不同）。
//!
//! ⚠ 分层如实：`非整数` / `负数` 在**工具面**就被拦下（`arg_u64` 与 `ParamSpec::accepts("integer")`）
//! ⇒ 写成判据 6 里"`json_type` 必须是 integer"（与 `ADR-0036`..`ADR-0041` 同一条分工）。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::drum::DrumVoice;
use yeban_theory::progression::Meter;
use yeban_theory::{default_backbeat, felt_beats_per_bar};

/// 夹具的小节数（与密度/声部节奏/音域/力度/跳进/摇摆六票同一份夹具）。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4。
const STYLE: &str = "synthwave";
/// 另一拍号的夹具（判据 5）：3/4 ⇒ 上界与缺省都必须随拍号移动。
const STYLE_34: &str = "waltz";
/// 夹具的调。
const KEY: &str = "C minor";
/// 夹具的进行（`waltz` 的 `typical_progressions` 里就有它 ⇒ 两个夹具共用）。
const PROGRESSION: &str = "i-VI-III-VII";
/// `synthwave` 那一份拍号**能被 felt 整除**的拍数（⛔ 在本文件抄 `4`）。
const FELTS: u8 = felt_beats_per_bar(Meter::COMMON);
/// 3/4 夹具的拍数（⛔ 抄 `3`）—— 判据 5 的上界读数就取它。
const FELTS_34: u8 = felt_beats_per_bar(Meter::WALTZ);
/// theory 的派生缺省（⛔ 抄 `2`）⇒ 判据 1 的"键缺席 ≡ 给派生值"用它。
const DERIVED: u64 = default_backbeat(Meter::COMMON) as u64;
/// 下界端点：`1` = 每一拍都打军鼓（⛔ 是"不打"）。
const EVERY_BEAT: u64 = 1;
/// 上界端点：4/4 ⇒ `4` = 只打第 0 拍。
const DOWNBEAT_ONLY: u64 = FELTS as u64;
/// 一拍多长（tick）：`ticks_per_bar / 拍数` —— 与 theory 数拍子的同一除法。
const TICKS_PER_BEAT: u64 = Meter::COMMON.ticks_per_bar() / FELTS as u64;
/// 军鼓的 GM 键（⛔ 抄 `38`）：判据靠它把鼓轨分成"军鼓"与"别的鼓件"。
const SNARE: u8 = DrumVoice::Snare.gm_key();

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律 + 夹具进行，本键不给）。
#[derive(Clone, Copy)]
struct Given<'a> {
    style_preset: &'a str,
    material_source: Option<&'a str>,
    include_drums: bool,
    include_melody: bool,
    backbeat: Option<u64>,
}

impl<'a> Given<'a> {
    fn theory() -> Self {
        Self {
            style_preset: STYLE,
            material_source: Some("theory"),
            include_drums: true,
            include_melody: true,
            backbeat: None,
        }
    }

    fn style(mut self, style_preset: &'a str) -> Self {
        self.style_preset = style_preset;
        self
    }

    fn backbeat(mut self, value: Option<u64>) -> Self {
        self.backbeat = value;
        self
    }

    fn material_source(mut self, value: Option<&'a str>) -> Self {
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

fn try_plan(given: Given<'_>) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        given.style_preset,
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
            swing_permille: None,
            drum_backbeat: given.backbeat,
            drum_style: None,
            melody_seed: None,
            beat_grouping: None,
        },
    )
}

fn plan(given: Given<'_>) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "`{}` 的 theory 规划（drumBackbeat={:?}）必须成功: {fault:?}",
            given.style_preset, given.backbeat
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

/// 鼓轨里**非军鼓**击点的形状多重集（排序后比较 —— ⛔ 用 id 比：本键会改条数，
/// 而 id 由原始 hit 序号派生，序号会跟着移）。
fn other_drum_shapes(drums: &BTreeMap<String, Note>) -> Vec<Note> {
    let mut shapes: Vec<Note> = drums
        .values()
        .filter(|note| note.2 != SNARE)
        .copied()
        .collect();
    shapes.sort();
    shapes
}

/// 军鼓击点的条数。
fn snare_count(drums: &BTreeMap<String, Note>) -> usize {
    drums.values().filter(|note| note.2 == SNARE).count()
}

/// 军鼓落在的**拍序号集合**（第 0 基、已对一小节的拍数取模）。
fn snare_beats(drums: &BTreeMap<String, Note>) -> BTreeSet<u64> {
    drums
        .values()
        .filter(|note| note.2 == SNARE)
        .map(|note| (note.0 / TICKS_PER_BEAT) % u64::from(FELTS))
        .collect()
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0043` D7 继承 `ADR-0038` D4 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
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

/// 断言一次"点名本键 + 回显取值 + 给出该拍号的界"的越界拒绝（判据 3 与判据 5 共用）。
fn assert_rejects_backbeat(style_preset: &str, raw: u64, felt_beats: u8) {
    let fault = try_plan(Given::theory().style(style_preset).backbeat(Some(raw))).expect_err(
        "越界不得静默夹紧，也⛔ 让 theory 的 `Ok(None)` 变成\"拍号推不出鼓组型\"（D6）",
    );
    assert_shape_reject(
        &fault,
        &format!("`{style_preset}` 的 `drumBackbeat` 越界 ({raw})"),
    );
    let payload = data(&fault);
    assert_eq!(
        payload["field"],
        json!("drumBackbeat"),
        "载荷必须点名是哪个键: {payload}"
    );
    assert_eq!(
        payload["value"],
        json!(raw),
        "载荷必须回显被拒的值: {payload}"
    );
    assert_eq!(
        payload["range"],
        json!([EVERY_BEAT, u64::from(felt_beats)]),
        "`{style_preset}` 的上界必须 = 现读 `felt_beats_per_bar` 的那个拍数（⛔ 本层第二张表）: {payload}"
    );
}

/// ⭐ 判据 1（`ADR-0043` D1/D5）：**键缺席 ≡ 显式给派生值** ⇒ 整个规划逐字节相同
/// （`SectionPlan: PartialEq`）。实现上缺席走的是 `GenreRule::drum_pattern` 薄封装那一支
/// （`ADR-0041` D5 的同一形状），给派生值走的是 `swung_styled_drum_pattern` 那一支 ⇒
/// 这条等价是**实测**而⛔ 定义（文件头）。顺带钉住夹具形状：鼓轨多条、军鼓在册、和声声部在场。
#[test]
fn absent_backbeat_is_byte_identical_to_the_derived_beat() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().backbeat(Some(DERIVED)));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"drumBackbeat\": {DERIVED}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let drums = &table["Drums"];
    assert!(
        drums.len() > 1,
        "夹具的鼓轨必须有多于一音符: {}",
        drums.len()
    );
    assert!(
        snare_count(drums) > 0,
        "夹具必须真的产出军鼓击点（否则判据 2 的\"军鼓随值变\"是空判据）"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert!(
            table[*part].len() > 1,
            "夹具的和声声部 `{part}` 必须有多于一层和弦"
        );
    }
}

/// ⭐ 判据 2（D1 的映射面 + "只改军鼓的落点与条数"）：给下界 `1` 与上界 `拍数` ⇒
/// * **非军鼓**击点的 `(起点, 时值, 音色, 力度)` 多重集与缺省跑**全等**（底鼓/踩镲/吊镲逐条不变）；
/// * 军鼓条数 = 每小节拍数 × 小节数（`1` = 每一拍）对上界那一支 = 小节数（只打第 0 拍），
///   缺省那一支恰为两者之间（读数 1：本键会加／筛击点）；
/// * 军鼓**落点拍集合** = `{0,1,2,3}` / `{0,2}` / `{0}`（读数 3：条数⛔ 足以判定）；
/// * 和声各声部与旋律与缺省跑**逐字节**相同（D3 的消费面）。
#[test]
fn the_backbeat_moves_only_the_snare_line() {
    let base = voices(&plan(Given::theory()));
    let base_drums = &base["Drums"];

    for (raw, beats) in [
        (EVERY_BEAT, &[0u64, 1, 2, 3][..]),
        (DERIVED, &[0u64, 2][..]),
        (DOWNBEAT_ONLY, &[0u64][..]),
    ] {
        let moved = voices(&plan(Given::theory().backbeat(Some(raw))));
        let moved_drums = &moved["Drums"];
        assert_eq!(
            base.keys().collect::<Vec<_>>(),
            moved.keys().collect::<Vec<_>>(),
            "军鼓落点 {raw}：声部集合必须不变"
        );
        for part in preset_parts(STYLE).expect("夹具必须在册") {
            assert_eq!(
                base[*part], moved[*part],
                "军鼓落点 {raw}：和声声部 `{part}` 必须与缺省跑逐条相同（本键只作用于鼓轨）"
            );
        }
        assert_eq!(
            base["Melody"], moved["Melody"],
            "军鼓落点 {raw}：旋律声部必须与缺省跑逐条相同"
        );
        assert_eq!(
            other_drum_shapes(base_drums),
            other_drum_shapes(moved_drums),
            "军鼓落点 {raw}：非军鼓击点的 (起点, 时值, 音色, 力度) 必须逐条不变"
        );
        assert_eq!(
            snare_beats(moved_drums),
            beats.iter().copied().collect::<BTreeSet<u64>>(),
            "军鼓落点 {raw}：落点拍集合必须点名到拍（⛔ 只看条数）"
        );
    }

    // 条数（单位 = 条）：`1` ⇒ 每一拍，上界 ⇒ 每小节一条，缺省夹在两档之间。
    let moved_every = voices(&plan(Given::theory().backbeat(Some(EVERY_BEAT))));
    let moved_last = voices(&plan(Given::theory().backbeat(Some(DOWNBEAT_ONLY))));
    let every_count = snare_count(&moved_every["Drums"]);
    let base_count = snare_count(&base["Drums"]);
    let last_count = snare_count(&moved_last["Drums"]);
    assert_eq!(
        every_count,
        usize::try_from(BARS * u64::from(FELTS)).expect("夹具条数必须落进 usize"),
        "`1` = 每一拍都打 ⇒ 军鼓条数 = 拍数 × 小节数"
    );
    assert_eq!(
        last_count,
        usize::try_from(BARS).expect("夹具条数必须落进 usize"),
        "上界 `{DOWNBEAT_ONLY}` ⇒ 每小节只打第 0 拍"
    );
    assert!(
        every_count > base_count && base_count > last_count,
        "军鼓条数必须随落点单调改变（实测 16 ↔ 8 ↔ 4）: {every_count} / {base_count} / {last_count}"
    );
}

/// ⭐ 判据 3（D2 的形状面，4/4 夹具）：越界 ⇒ 点名是哪个键、载荷带 `value` + `range:[1,4]`
/// （`4` = 现读 `felt_beats_per_bar(4/4)`）、⛔ `reason` 键；两个端点 `1` / `4` 必须放行。
/// ⚠ `0` 与"超拍数"都**必须**是本层的参数拒绝（载荷带 `field`）：theory 对这两种越界返回
/// `Ok(None)`（⛔ 报错、⛔ 钳制，读数 4），放下去只会得到兜底分支那句不点名本键的话。
#[test]
fn out_of_range_is_rejected_naming_the_key() {
    for raw in [0u64, 5, 256, 100_000] {
        assert_rejects_backbeat(STYLE, raw, FELTS);
    }
    for bound in [EVERY_BEAT, DOWNBEAT_ONLY] {
        try_plan(Given::theory().backbeat(Some(bound))).unwrap_or_else(|fault| {
            panic!(
                "`drumBackbeat` = {bound} 在 `1..={}` 内，必须放行: {fault:?}",
                FELTS
            )
        });
    }
}

/// ⭐ 判据 4（D3 的消费面，**两向都钉**）：本键要求材料源 = theory **且**鼓轨在场；
/// 反钉"⛔ 把旋律那条更严的要求扩散过来" —— `includeMelody:false` 时给本键必须放行。
#[test]
fn the_backbeat_key_requires_the_theory_source_and_the_drum_part() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .backbeat(Some(EVERY_BEAT)),
        )
        .expect_err("`drumBackbeat` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`drumBackbeat` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    // 鼓轨不在场 ⇒ 本键无人消费，必须响亮拒绝而不是静默忽略。
    let fault = try_plan(Given::theory().drums(false).backbeat(Some(EVERY_BEAT)))
        .expect_err("`drumBackbeat` 不得在没有鼓轨时静默失效");
    assert_shape_reject(&fault, "`drumBackbeat` 脱离 includeDrums");
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeDrums")),
        "载荷必须指出要补 includeDrums: {}",
        data(&fault)
    );
    // 反钉：没有旋律声部时必须放行（军鼓落点只被鼓轨消费）。
    try_plan(Given::theory().melody(false).backbeat(Some(EVERY_BEAT))).unwrap_or_else(|fault| {
        panic!("`drumBackbeat` ⛔ 要求旋律声部在场（它与 swingPermille/drumDensity 同一支口径）: {fault:?}")
    });
}

/// ⭐ 判据 5（D1/D2 的"界随拍号移动"那一支）：3/4 夹具上同一个数的**合法性**与 `range`
/// 里的上界都随 `felt_beats_per_bar` 移动 —— 这是本键与 `swingPermille`（常量对）的
/// **结构性差别**，也是"本层⛔ 抄一张「拍号 → 拍数」表、界必须来自 theory 函数"的唯一防线：
/// 把上界写死成 4，这一条就会在 3/4 上红（`4` 被放行而契约说该拒）。
#[test]
fn the_bound_moves_with_the_meter() {
    // `4` 在 4/4 上合法（判据 3 已钉放行），在 3/4 上必须被拒且上界报 3；`0` 一并钉住。
    for raw in [0u64, u64::from(FELTS_34) + 1] {
        assert_rejects_backbeat(STYLE_34, raw, FELTS_34);
    }
    for raw in EVERY_BEAT..=u64::from(FELTS_34) {
        try_plan(Given::theory().style(STYLE_34).backbeat(Some(raw))).unwrap_or_else(|fault| {
            panic!(
                "`{STYLE_34}` 的 `drumBackbeat` = {raw} 在 `1..={}` 内，必须放行: {fault:?}",
                FELTS_34
            )
        });
    }
    // 键缺席在两个拍号上都必须放行（缺省 = 读派生值，⛔ 随拍号写死一个数）。
    for style_preset in [STYLE, STYLE_34] {
        try_plan(Given::theory().style(style_preset)).unwrap_or_else(|fault| {
            panic!("`{style_preset}` 的缺省鼓轨（不给本键）必须放行: {fault:?}")
        });
    }
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON）。
/// ⭐ 契约⛔ 登记 `default`（派生值随拍号移动）也⛔ 登记 `maximum`（上界 = 该拍号的拍数，
/// 写死会对 2/4 那条流派报出"契约说 5 能、本层拒"的矛盾）—— 这是 `ADR-0043` D1 里
/// 与 `ADR-0041` 判据 6 **不同**的一处（那一个钉 `minimum`/`maximum` 两个数字）。
/// 类型必须是 integer（工具面的"非整数 / 负数"由这道类型闸承担 —— 见文件头）。
#[test]
fn drum_backbeat_is_declared_on_both_param_surfaces_without_default_or_maximum() {
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
        .param("drumBackbeat")
        .expect("参数表必须声明 `drumBackbeat`");
    assert_eq!(param.json_type, "integer", "`drumBackbeat` 必须是整数参数");
    assert!(!param.required, "缺省 = 派生值 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0043") && param.doc.contains("拍序号"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );
    let property = &declared["drumBackbeat"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `drumBackbeat` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("integer"));
    assert_eq!(
        property["minimum"],
        json!(EVERY_BEAT),
        "下界不随拍号移动 ⇒ 必须登记 `minimum: 1`: {property}"
    );
    assert!(
        property.get("maximum").is_none(),
        "本键⛔ 写 `maximum`：上界 = 该流派拍号的拍数（4/4 是 4、2/4 是 2），\
         写一个固定数字会对窄拍号报出\"契约说能、本层拒\"的矛盾（`ADR-0043` D1）: {property}"
    );
    assert!(
        property.get("default").is_none(),
        "本键⛔ 写固定 `default`：缺省语义是\"不覆盖 theory 的派生值 `default_backbeat(meter)`\"，\
         而那一个数随拍号移动（实测 4/4→2、2/4 与 6/8→1、5/4→3）: {property}"
    );
}
