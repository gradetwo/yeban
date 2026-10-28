//! `ADR-0044`（缺口 `G8l`：鼓的底鼓口径是入参）：`yeban_propose_section` 的 `drumStyle` ——
//! 把 `GenreRule::drum_pattern` 参数表里**最后**一个写死的实参（`self.drum_style`）开放成可给的
//! 入参，缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **档位名**（⛔ 数字、⛔ 枚举下标）：`"metric"` = 底鼓落在**组起点**，
//! `"four-on-the-floor"` = 底鼓落在**每一个拍的起点**。界 = theory 公开的
//! `DrumStyle::ALL` × `name()`（现读两项），⛔ 在本文件抄字符串 ⇒ 下面的 `METRIC`/`FLOOR`
//! 都**从 theory 取**（`ADR-0044` D2 的同一条：出现第二份写者就是缺陷）。
//!
//! ⚠ **与 `ADR-0041`（摇摆）同族的两处、与 `ADR-0043`（军鼓落点）相反的一处**：
//! 1. 契约里**⛔ `default`** —— 现读 `GENRES` 182 条里 **169 条**登记 `Metric`、**13 条**登记
//!    `FourOnTheFloor`（`disco`/`house`/`techno`/`trance` 那一支）⇒ 缺省不是一个固定字符串（D1/D5）；
//!    夹具 `synthwave` 属 169 那一支 ⇒ 判据 1 的"缺席 ≡ 登记值"是**实测等价**而⛔ 定义。
//! 2. 界⛔ 随拍号移动 ⇒ 与 `drumBackbeat` 不同：那一个键⛔ 能写 `maximum`，这一个**可以**登记 `enum`
//!    （判据 6 拿 `DrumStyle::ALL` 现场对账）。
//! 3. 本键**只动底鼓**：实测 `182 流派 × onsets{4,8,16}` = 546 组里可测的 518 组，**非底鼓**击点的
//!    `(GM 键, tick, 重音)` 表两档**逐条相同**（不等 0 组）⇒ 判据 2 能量成"别的击点⛔ 变"，
//!    这是 `drumBackbeat`（会同时加/筛军鼓）⛔ 具备的不变量。
//!
//! ⚠ **"严格更多"⛔ 全局成立**（这条推翻"照抄 `ADR-0043` 判据 3"的写法）：探针实测 518 组里有
//! **6 组两档完全相同** —— `jig`/`irish_trad`/`celtic` 三条 **6/8** 流派 × `onsets ∈ {4, 8}`，
//! 因为复合二拍里两个拍起点恒是组起点 ⇒ 判据 2 在 **4/4 夹具内**钉"更多"、判据 5 在 **6/8 上**钉
//! "逐字节相同"。把任一支写成全平台断言都会留下一个假绿或一条Permanent红。
//!
//! ⚠ 分层如实：`非字符串` / 错类型在**工具面**就被拦下（`ParamSpec::accepts("string")`），
//! 纯领域层量不到 ⇒ 写成判据 6 里"`json_type` 必须是 string"（与 `ADR-0036`..`ADR-0043` 同一条分工）。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::drum::{DrumStyle, DrumVoice};
use yeban_theory::felt_beats_per_bar;
use yeban_theory::progression::Meter;

/// 夹具的小节数（与力度/音域/跳进/摇摆/军鼓五票同一份夹具）。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4、登记 `Metric` ⇒ 缺省 = 组起点底鼓。
const STYLE: &str = "synthwave";
/// 复合二拍的夹具（`ADR-0044` 读数 3 的**反例**那一支）：6/8 下两档恒同。
const STYLE_68: &str = "jig";
/// 夹具的调。
const KEY: &str = "C minor";
/// 夹具的进行。
const PROGRESSION: &str = "i-VI-III-VII";
/// 度量档位名（⭐ 从 theory 现读，⛔ 写字面量 `"metric"`）。
const METRIC: &str = DrumStyle::Metric.name();
/// 四踩档位名（⭐ 从 theory 现读，⛔ 写字面量 `"four-on-the-floor"`）。
const FLOOR: &str = DrumStyle::FourOnTheFloor.name();
/// 底鼓的 GM 键（= theory 的 `DrumVoice::Kick.gm_key()`，⛔ 抄 `36`）。
const KICK: u8 = DrumVoice::Kick.gm_key();
/// 平直档（= theory 的 `SWING_PERMILLE_STRAIGHT`，用于判据 2④ 的正交读数）。
const STRAIGHT: u64 = yeban_theory::SWING_PERMILLE_STRAIGHT as u64;
/// 一档中等摇摆（与 `ADR-0041` 的判据同一档，用于证明两轴互不接管）。
const MEDIUM: u64 = 580;
/// 夹具拍号的拍数（4/4 ⇒ 4）与每拍 tick 数（用于"新增底鼓恒在拍起点"）。
const FELTS: u8 = felt_beats_per_bar(Meter::COMMON);
const TICKS_PER_BEAT: u64 = Meter::COMMON.ticks_per_bar() / FELTS as u64;

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律 + 夹具进行，本键不给）。
#[derive(Clone, Copy)]
struct Given {
    material_source: Option<&'static str>,
    include_drums: bool,
    include_melody: bool,
    style: Option<&'static str>,
    swing_permille: Option<u64>,
    onsets_per_bar: Option<u64>,
    style_preset: &'static str,
}

impl Given {
    fn theory() -> Self {
        Self {
            material_source: Some("theory"),
            include_drums: true,
            include_melody: true,
            style: None,
            swing_permille: None,
            onsets_per_bar: None,
            style_preset: STYLE,
        }
    }

    fn style(mut self, value: Option<&'static str>) -> Self {
        self.style = value;
        self
    }

    fn swing(mut self, value: Option<u64>) -> Self {
        self.swing_permille = value;
        self
    }

    fn onsets(mut self, value: Option<u64>) -> Self {
        self.onsets_per_bar = value;
        self
    }

    fn preset(mut self, value: &'static str) -> Self {
        self.style_preset = value;
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
            onsets_per_bar: given.onsets_per_bar,
            velocity_min: None,
            velocity_max: None,
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: None,
            harmony_max_voice_jump: None,
            swing_permille: given.swing_permille,
            drum_backbeat: None,
            drum_style: given.style,
        },
    )
}

fn plan(given: Given) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "theory 规划（drumStyle={:?} @ {}）必须成功: {fault:?}",
            given.style, given.style_preset
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

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`。
type Note = (u64, u64, u8, u8);

/// 鼓轨里**非底鼓**击点的形状表（按 `(起点, 音高)` 升序 ⇒ 与 id 无关）。
///
/// ⚠ ⛔ 按 id 比：本键会**加**击点，而鼓音符 id = `theory-drum:{section}:{hit_index}:{voice}`
/// ⇒ `hit_index` 会因插入而整体位移（`ADR-0043` 读数 1 同一条口径）。
fn other_drum_shapes(table: &BTreeMap<String, Note>) -> Vec<Note> {
    let mut list: Vec<Note> = table
        .values()
        .filter(|note| note.2 != KICK)
        .copied()
        .collect();
    list.sort_unstable();
    list
}

/// 鼓轨里**底鼓**的起点集合（升序去重）。
fn kick_starts(table: &BTreeMap<String, Note>) -> Vec<u64> {
    let mut list: Vec<u64> = table
        .values()
        .filter(|note| note.2 == KICK)
        .map(|note| note.0)
        .collect();
    list.sort_unstable();
    list
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`，且载荷⛔ 带 `reason` 键
/// （`ADR-0044` D7 继承 `ADR-0038` D4 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
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

/// `DrumStyle::ALL` 的公开名字表（判据 3 与判据 6 共用的**唯一**期望值来源）。
fn accepted() -> Vec<&'static str> {
    DrumStyle::ALL.iter().map(|style| style.name()).collect()
}

/// ⭐ 判据 1（`ADR-0044` D1/D2 + `ADR-0016`）：键缺席 ≡ 显式给**该流派登记的档位**
/// ⇒ 整个规划逐字节相同（`SectionPlan: PartialEq`）。夹具 `synthwave` 属 169 条登记 `Metric`
/// 那一支（读数 1）⇒ 这条等价是**实测**而⛔ 定义。
/// 顺带钉住夹具的形状：底鼓在册（否则判据 2 是空判据）、和声声部在场。
#[test]
fn absent_style_is_byte_identical_to_the_registered_metric() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().style(Some(METRIC)));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"drumStyle\": \"{METRIC}\"}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let drums = &table["Drums"];
    assert!(
        drums.len() > 1,
        "夹具的鼓轨必须有多于一音符: {}",
        drums.len()
    );
    // 夹具必须真的产出底鼓（否则判据 2 的"更多"是空判据）。
    assert!(
        !kick_starts(drums).is_empty(),
        "夹具的鼓轨必须有底鼓击点: {drums:?}"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert!(
            table[*part].len() > 1,
            "夹具的和声声部 `{part}` 必须有多于一层和弦"
        );
    }
}

/// ⭐ 判据 2（D1 的映射面 + "只动底鼓" + 与摇摆正交）：在 4/4 夹具上给 `FLOOR` ⇒
/// * **非底鼓**击点的形状表逐条与缺省跑相同（读数 2：本键⛔ 改踩镲/军鼓/吊镲，⛔ 改力度与否由这条兜住）；
/// * 底鼓起点集合是缺省的**超集**且**严格更多**（实测 `8 → 16` 条）；
/// * **新增**的那批起点恒是拍起点（`tick % TICKS_PER_BEAT == 0`，读数 4 的形状面）；
/// * 和声各声部与旋律与缺省跑**逐字节**相同（D3 的消费面）；
/// * 同一个 `drumStyle` 在 `swingPermille = {STRAIGHT}` 与 `{MEDIUM}` 两档下的底鼓集合**相同**
///   （读数 4：两轴正交 ⇒ 这条能挡"把 style 接到 swing 的实参上"那一类接错）。
#[test]
fn four_on_the_floor_only_adds_kicks_on_beat_starts() {
    let base = plan(Given::theory());
    let base_table = voices(&base);
    let added = plan(Given::theory().style(Some(FLOOR)));
    let added_table = voices(&added);

    assert_eq!(
        base_table.keys().collect::<Vec<_>>(),
        added_table.keys().collect::<Vec<_>>(),
        "口径 `{FLOOR}`：声部集合必须不变"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert_eq!(
            base_table[*part], added_table[*part],
            "口径 `{FLOOR}`：和声声部 `{part}` 必须与缺省跑逐条相同（本键只作用于鼓轨）"
        );
    }
    assert_eq!(
        base_table["Melody"], added_table["Melody"],
        "口径 `{FLOOR}`：旋律声部必须与缺省跑逐条相同"
    );

    let base_drums = &base_table["Drums"];
    let added_drums = &added_table["Drums"];
    assert_eq!(
        other_drum_shapes(base_drums),
        other_drum_shapes(added_drums),
        "口径 `{FLOOR}`：**非底鼓**击点的 (起点, 时值, 音高, 力度) 表必须逐条相同（读数 2：518 组实测⛔ 一组不等）"
    );

    let base_kicks = kick_starts(base_drums);
    let added_kicks = kick_starts(added_drums);
    assert!(
        added_kicks.len() > base_kicks.len(),
        "4/4 夹具上 `{FLOOR}` 的底鼓必须严格多于 `{METRIC}`（实测 8 → 16 条）：{} vs {}",
        base_kicks.len(),
        added_kicks.len()
    );
    for start in &base_kicks {
        assert!(
            added_kicks.contains(start),
            "`{FLOOR}` 的击点集合必须是 `{METRIC}` 的**超集**（theory 的文档口径）: 缺 {start}"
        );
    }
    for start in &added_kicks {
        assert_eq!(
            start % TICKS_PER_BEAT,
            0,
            "`{FLOOR}` 新增的底鼓恒落在**拍起点**（`{FELTS}` 拍/小节、每拍 {TICKS_PER_BEAT} tick）: {start}"
        );
    }

    // 正交读数：style ⛔ 被 swing 接管，swing 也⛔ 改变底鼓落点。
    let straight = voices(&plan(
        Given::theory().style(Some(FLOOR)).swing(Some(STRAIGHT)),
    ));
    let swung = voices(&plan(
        Given::theory().style(Some(FLOOR)).swing(Some(MEDIUM)),
    ));
    assert_eq!(
        kick_starts(&straight),
        kick_starts(&swung),
        "两档摇摆下的底鼓起点集合必须相同（摇摆只推移每对格位里的**第二个** onset，\
         而 `{FLOOR}` 的落点恒在拍起点 —— 读数 4 用它挡\"接错实参\"）"
    );
}

/// ⭐ 判据 3（D2 的形状面）：未知取值 ⇒ 点名是哪个键、载荷带 `value` + `accepted`
/// （⛔ `reason` 键），且 `accepted` **恰等于** `DrumStyle::ALL × name()` 现读的那两项；
/// 两个合法值都必须放行（⛔ 夹紧、⛔ 静默降级成 `Metric`）。
#[test]
fn an_unknown_style_is_rejected_naming_the_key_and_the_accepted_set() {
    for raw in [
        "four",
        "",
        "FOUR-ON-THE-FLOOR",
        "FourOnTheFloor",
        "四踩",
        "normal",
    ] {
        let fault = try_plan(Given::theory().style(Some(raw)))
            .expect_err("未知档位不得静默降级（`ADR-0036` D3 的同一条口径）");
        assert_shape_reject(&fault, &format!("`drumStyle` = {raw:?}"));
        let payload = data(&fault);
        assert_eq!(
            payload["field"],
            json!("drumStyle"),
            "载荷必须点名是哪个键: {payload}"
        );
        assert_eq!(
            payload["value"],
            json!(raw),
            "载荷必须回显被拒的值: {payload}"
        );
        let expected: Vec<&str> = accepted();
        assert_eq!(
            payload["accepted"],
            json!(expected),
            "载荷给出的候选必须**现场**取自 `DrumStyle::ALL`（⛔ 第二份字符串表）: {payload}"
        );
    }
    for bound in accepted() {
        try_plan(Given::theory().style(Some(bound))).unwrap_or_else(|fault| {
            panic!("`drumStyle` = {bound:?} 是 `DrumStyle::ALL` 里的一项，必须放行: {fault:?}")
        });
    }
}

/// ⭐ 判据 4（D3 的消费面，**两向都钉**）：本键要求材料源 = theory **且**鼓轨在场；
/// 反钉"⛔ 把旋律那条更严的要求扩散过来" —— `includeMelody:false` 时给本键必须放行。
#[test]
fn the_style_key_requires_the_theory_source_and_the_drum_part() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory()
                .material_source(material_source)
                .style(Some(FLOOR)),
        )
        .expect_err("`drumStyle` 不得脱离 materialSource:\"theory\" 单独给");
        assert_shape_reject(&fault, "`drumStyle` 脱离 theory");
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    // 鼓轨不在场 ⇒ 本键无人消费，必须响亮拒绝而不是静默忽略。
    let fault = try_plan(Given::theory().drums(false).style(Some(FLOOR)))
        .expect_err("`drumStyle` 不得在没有鼓轨时静默失效");
    assert_shape_reject(&fault, "`drumStyle` 脱离 includeDrums");
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeDrums")),
        "载荷必须指出要补 includeDrums: {}",
        data(&fault)
    );
    // 反钉：没有旋律声部时必须放行（底鼓落点只被鼓轨消费）。
    try_plan(Given::theory().melody(false).style(Some(FLOOR))).unwrap_or_else(|fault| {
        panic!("`drumStyle` ⛔ 要求旋律声部在场（与 swingPermille/drumDensity/drumBackbeat 同一支口径）: {fault:?}")
    });
}

/// ⭐ 判据 5（读数 3 的**反例**那一支，⛔ 照抄 `ADR-0043` 判据 3 的地方就在这里）：
/// 复合二拍（6/8 的 `jig`）上两个档位的 `SectionPlan` **逐字节相同** —— 因为两个拍起点恒是组起点。
/// 这条同时钉住两件事：①⛔ 把"给 `FLOOR` 必然更多底鼓"写成全平台断言（那是 512/518 的事实，
/// ⛔ 是 518/518）；②⛔ 让本层去判可行性 —— 同一夹具把网格压到 `onsets = 16` 时，
/// theory 的 `ProgressionTooDense` 在**两个档位**下必须是**同一份**报文（读数 5）。
#[test]
fn the_compound_duple_meter_makes_the_two_styles_identical() {
    let metric = plan(Given::theory().preset(STYLE_68).style(Some(METRIC)));
    let floor = plan(Given::theory().preset(STYLE_68).style(Some(FLOOR)));
    assert_eq!(
        metric, floor,
        "`{STYLE_68}`（6/8）上两档必须逐字节相同（实测 6 组反例：jig/irish_trad/celtic × onsets 4/8）\
         —— 这一条存在的全部理由就是⛔ 把判据 2 的\"严格更多\"外推到全平台"
    );
    // 缺省（不给本键）也与两档相同 ⇒ 夹具的登记值就是 `Metric` 那一支。
    assert_eq!(
        plan(Given::theory().preset(STYLE_68)),
        floor,
        "`{STYLE_68}` 的缺省（⛔ 本键）必须与该流派登记值逐字节相同（`ADR-0016`）"
    );

    // 可行性⛔ 随档位变：两档在 `onsets = 16` 上都失败，且报文**逐字相同**（D6 原样转发）。
    let dense = 16u64;
    let metric_fault = try_plan(
        Given::theory()
            .preset(STYLE_68)
            .onsets(Some(dense))
            .style(Some(METRIC)),
    )
    .expect_err("`{STYLE_68}` 在 onsets=16 上必须失败（theory 的 ProgressionTooDense）");
    let floor_fault = try_plan(
        Given::theory()
            .preset(STYLE_68)
            .onsets(Some(dense))
            .style(Some(FLOOR)),
    )
    .expect_err("同一份失败必须在 `{FLOOR}` 档下同样发生（本键⛔ 参与可行性判定）");
    assert_eq!(
        format!("{metric_fault:?}"),
        format!("{floor_fault:?}"),
        "两档的失败报文必须逐字相同 ⇒ 本层⛔ 判「该口径在这拍号/格位下产不产得出鼓点」（D6）"
    );
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON）。
/// ⭐ 契约里**⛔ `default`**（读数 1：169 条 `Metric` / 13 条 `FourOnTheFloor` ⇒ 一个固定字符串
/// 会把两支说成一支），但 `enum` **可以**登记 —— 它⛔ 随拍号移动（与 `ADR-0043`⛔ 写 `maximum`
/// 是**不同**的一处）。登记的 `enum` 必须**恰等于** `DrumStyle::ALL × name()`（⛔ 第二份字符串表）。
/// 类型必须是 string（工具面的"非字符串"由这道类型闸承担 —— 见文件头）。
#[test]
fn drum_style_is_declared_on_both_param_surfaces_with_the_theory_names_and_no_default() {
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
    let param = spec.param("drumStyle").expect("参数表必须声明 `drumStyle`");
    assert_eq!(param.json_type, "string", "`drumStyle` 必须是字符串参数");
    assert!(!param.required, "缺省 = 该流派登记值 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0044") && param.doc.contains("底鼓"),
        "参数说明必须写清口径与出处: {}",
        param.doc
    );
    let property = &declared["drumStyle"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `drumStyle` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("string"));
    let expected: Vec<&str> = accepted();
    assert_eq!(
        property["enum"],
        json!(expected),
        "契约的候选必须与 theory 公开的 `DrumStyle::ALL × name()` 是**同一组字符串**\
         （⛔ 两份表）: {property}"
    );
    assert!(
        property.get("default").is_none(),
        "本键⛔ 写固定 `default`：现读 `GENRES` 里 169 条登记 `{METRIC}`、13 条登记 `{FLOOR}`，\
         缺省语义是\"不覆盖该流派登记值\"（`ADR-0044` D1/D5）: {property}"
    );
}
