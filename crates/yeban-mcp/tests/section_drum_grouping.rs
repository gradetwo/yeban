//! `ADR-0046`（缺口 `G8n`：拍内分组是入参）：`yeban_propose_section` 的 `beatGrouping` ——
//! 把 `swung_styled_drum_pattern` 里**写死为 `None`** 的那个实参（`grouping`）开放成可给的入参，
//! 缺省 ⇒ 逐字节不变。
//!
//! 单位口径 = **拍数序列**（逐项 = 该组的拍数；语义 = 组起点是每一组的第 0 拍），
//! ⛔ tick、⛔ "每小节打几下"、⛔ 小节号。
//!
//! ⚠ **本键的合法形状随拍号移动**（`ADR-0046` D2/D5）：逐项 `>= 1` 且总和必须等于该拍号的
//! `felt_beats_per_bar(meter)`（现读：4/4 ⇒ 4、3/4 ⇒ 3、6/8 ⇒ 2）。合法性由 theory 的
//! `BeatGrouping::new` **判**（本层只做 `u8` 的类型面转换）⇒ 契约里因此⛔ 写 `default`、
//! ⛔ 写逐项 `maximum`、也⛔ 写 `maxItems`。判据 5 钉这一支：`[3, 2]` 在 5/4 上合法、在 4/4 上被拒。
//!
//! ⚠ **本键与鼓线其余四根轴不同的一面**（与本文件判据 2 的读数）：它改的是**分派与重量** ——
//! 组起点决定底鼓（Metric 口径）与吊镲的落点，`weight` 取 `metric_weight_in` 与
//! `metric_weight_grouped` 的较大者 ⇒ `hit.accent` 跟着变 ⇒ **力度档也变**；而**踩镲的条数与
//! 起点逐条不变**（网格不动）。因此 `G8i` 那句"鼓轨 id 集合逐条不变"在此⛔ 能用（底鼓条数会变，
//! 而 id 由**原始 hit 序号**派生），本文件比的是**形状**与**落点拍集合**。
//!
//! ⚠ 分层如实：`非数组` / `负数` / 非整数在**工具面**就被拦下（`arg_u64_list` 与
//! `ParamSpec::accepts`）⇒ 写成判据 6 里"`json_type` 必须是 array"。`0` 与 `> 255` 落在本层
//! 的类型面（`u8` 表达不了 / theory 判零非法），`总和 != 拍数` 落在 theory 的判定面上。

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::drum::DrumVoice;
use yeban_theory::felt_beats_per_bar;
use yeban_theory::progression::Meter;

/// 夹具的小节数（与鼓线其余四票同一份夹具）。
const BARS: u64 = 4;
/// 夹具的风格预设：4/4。
const STYLE: &str = "synthwave";
/// 另一拍号的夹具（判据 5）：3/4 ⇒ 合法形状与 `beatsPerBar` 都必须随拍号移动。
const STYLE_34: &str = "waltz";
/// 夹具的调。
const KEY: &str = "C minor";
/// 夹具的进行（`waltz` 的 `typical_progressions` 里就有它 ⇒ 两个夹具共用）。
const PROGRESSION: &str = "i-VI-III-VII";
/// `synthwave` 的拍数（⛔ 在本文件抄 `4`）—— 判据 3/5 的 `beatsPerBar` 读数取它。
const FELTS: u8 = felt_beats_per_bar(Meter::COMMON);
/// 3/4 夹具的拍数（⛔ 抄 `3`）。
const FELTS_34: u8 = felt_beats_per_bar(Meter::WALTZ);
/// 把内置口径**逐位复现**的那条分组（`rhythm.rs` 的 `BeatGrouping` 文档：4/4 ⇒ `[2, 2]`）
/// ⇒ 判据 1 的"键缺席 ≡ 给这条"用它。
const DERIVED: [u64; 2] = [2, 2];
/// 3/4 的同一条口径（`rhythm.rs` 现读：3/4 ⇒ `[3]`）⇒ 判据 5 用它在**第二个拍号**上再钉一次
/// "缺省 ≡ 该拍号的复现分组"（⛔ 把 `[2, 2]` 当成跨拍号常量）。
const DERIVED_34: [u64; 1] = [3];
/// 4/4 的另一个合法宽度：**每一拍**单独成组 ⇒ 4 个组起点（判据 2）。
const EVERY_BEAT: [u64; 4] = [1, 1, 1, 1];
/// 4/4 的**单一组**（其余一切取值的另一端）⇒ 判据 3 的放行端点之一。
const ONE_GROUP: [u64; 1] = [4];
/// 缺省力度阶梯的两档（`ADR-0037`：⛔ 抄别的数字 —— 判据 2 用它读"重音档变没变"）。
const ACCENT: u8 = 112;
/// 缺省力度阶梯的下档。
const THEORY_TIER: u8 = 96;
/// 三件鼓件的 GM 键（⛔ 抄 `36/38/42`）：判据靠它们把鼓轨分成底鼓 / 军鼓 / 踩镲。
const KICK: u8 = DrumVoice::Kick.gm_key();
const SNARE: u8 = DrumVoice::Snare.gm_key();
const HAT: u8 = DrumVoice::HiHat.gm_key();

/// 一条音符的可读数形状：`(起点 tick, 时值 tick, 音高, 力度)`，按**音符 id** 索引。
type Note = (u64, u64, u8, u8);

/// 一次请求的可给项（缺省 = theory + 鼓 + 旋律 + 夹具进行，本键不给）。
#[derive(Clone, Copy)]
struct Given<'a> {
    style_preset: &'a str,
    material_source: Option<&'a str>,
    include_drums: bool,
    include_melody: bool,
    grouping: Option<&'a [u64]>,
}

impl<'a> Given<'a> {
    fn theory() -> Self {
        Self {
            style_preset: STYLE,
            material_source: Some("theory"),
            include_drums: true,
            include_melody: true,
            grouping: None,
        }
    }

    fn style(mut self, style_preset: &'a str) -> Self {
        self.style_preset = style_preset;
        self
    }

    fn grouping(mut self, value: Option<&'a [u64]>) -> Self {
        self.grouping = value;
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
            drum_backbeat: None,
            drum_style: None,
            beat_grouping: given.grouping,
            melody_seed: None,
            harmony_seed: None,
            material_clip_ids: None,
            part_tracks: None,
        },
    )
}

fn plan(given: Given<'_>) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "`{}` 的 theory 规划（beatGrouping={:?}）必须成功: {fault:?}",
            given.style_preset, given.grouping
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

/// 某一鼓件的**形状多重集**：`(起点 tick, 时值 tick, 音高, 力度)`，排序后比较
/// （⛔ 用 id 比：本键会改底鼓条数，而 id 由原始 hit 序号派生）。
fn shapes_of(drums: &BTreeMap<String, Note>, voice: u8) -> Vec<Note> {
    let mut shapes: Vec<Note> = drums
        .values()
        .filter(|note| note.2 == voice)
        .copied()
        .collect();
    shapes.sort();
    shapes
}

/// 某一鼓件的**网格面**形状：`(起点 tick, 时值 tick, 音高)` —— 去掉力度后的"网格有没有动"。
fn grid_of(drums: &BTreeMap<String, Note>, voice: u8) -> Vec<(u64, u64, u8)> {
    let mut grid: Vec<(u64, u64, u8)> = drums
        .values()
        .filter(|note| note.2 == voice)
        .map(|note| (note.0, note.1, note.2))
        .collect();
    grid.sort();
    grid
}

/// 某一鼓件落在的**拍序号集合**（第 0 基、已对一小节的拍数取模）。
fn beats_of(drums: &BTreeMap<String, Note>, voice: u8) -> BTreeSet<u64> {
    let ticks_per_beat = Meter::COMMON.ticks_per_bar() / u64::from(FELTS);
    drums
        .values()
        .filter(|note| note.2 == voice)
        .map(|note| (note.0 / ticks_per_beat) % u64::from(FELTS))
        .collect()
}

/// 鼓轨里落在**重音档**（`ACCENT`）的击点条数 —— 分组改 `weight` 的读数面。
fn accent_count(drums: &BTreeMap<String, Note>) -> usize {
    drums.values().filter(|note| note.3 == ACCENT).count()
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 断言一条拒绝：码 = `INVALID_PARAMETER_RANGE`、载荷点名本键、⛔ `reason` 键
/// （`ADR-0046` D7 继承 `ADR-0043` D7 ⇒ `fault_vocabulary` 的词表与规范面摘要不变）。
/// `beats` = `Some(n)` 时要求载荷给出 `beatsPerBar: n`（语义面拒绝）；
/// `None` 时只要求点名（类型面：`> 255` 无法用 `u8` 表达）。
fn assert_rejects_grouping(style_preset: &str, raw: &[u64], beats: Option<u8>) {
    let fault = try_plan(Given::theory().style(style_preset).grouping(Some(raw)))
        .expect_err("不合法的分组不得静默夹紧，也⛔ 落到兜底分支那句\"拍号推不出鼓组型\"（D6）");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "`{style_preset}` 的 `beatGrouping` = {raw:?} 必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    let payload = data(&fault);
    assert!(
        payload.get("reason").is_none(),
        "`{style_preset}` 的 `beatGrouping` = {raw:?}：载荷⛔ 带 `reason` 键: {payload}"
    );
    assert_eq!(
        payload["field"],
        json!("beatGrouping"),
        "载荷必须点名是哪个键: {payload}"
    );
    assert_eq!(
        payload["value"],
        json!(raw),
        "载荷必须回显被拒的取值: {payload}"
    );
    match beats {
        None => assert!(
            payload.get("beatsPerBar").is_none(),
            "类型面拒绝（`u8` 表达不了）⛔ 该报 `beatsPerBar`: {payload}"
        ),
        Some(beats) => assert_eq!(
            payload["beatsPerBar"],
            json!(beats),
            "载荷必须给出**该拍号**的拍数（⛔ 本层第二张表）: {payload}"
        ),
    }
}

/// ⭐ 判据 1（`ADR-0046` D1/D4）：**键缺席 ≡ 显式给"把内置口径逐位复现"的那条分组**
/// ⇒ 整个规划逐字节相同（`SectionPlan: PartialEq`）。
///
/// 实现上缺席走的是 `GenreRule::drum_pattern` 薄封装那一支（它内部传 `None`），
/// 给 `[2, 2]` 走的是 `swung_styled_drum_pattern` 那一支 ⇒ 这条等价是**实测**而⛔ 定义。
/// 顺带钉住夹具形状：鼓轨多条、三件鼓件都在册、和声声部在场。
#[test]
fn absent_grouping_is_byte_identical_to_the_derived_split() {
    let absent = plan(Given::theory());
    let explicit = plan(Given::theory().grouping(Some(&DERIVED)));
    assert_eq!(
        absent, explicit,
        "键缺席必须逐字节等于给 `{{\"beatGrouping\": {DERIVED:?}}}`（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    let drums = &table["Drums"];
    assert!(
        drums.len() > 1,
        "夹具的鼓轨必须有多于一音符: {}",
        drums.len()
    );
    for voice in [KICK, SNARE, HAT] {
        assert!(
            !shapes_of(drums, voice).is_empty(),
            "夹具必须真的产出该鼓件的击点（否则判据 2 是空判据）: {voice}"
        );
    }
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert!(
            table[*part].len() > 1,
            "夹具的和声声部 `{part}` 必须有多于一层和弦"
        );
    }
}

/// ⭐ 判据 2（D1 的映射面 + "只改分派与重量，⛔ 改网格"）：4/4 夹具上给 `[1, 1, 1, 1]` ⇒
/// * **踩镲**的 `(起点, 时值, 音高)` 多重集与缺省跑**全等**（网格不动，读数：16 条/小节 ↔ 16 条/小节）；
/// * **军鼓**的形状多重集与缺省跑**全等**（落点由 `drumBackbeat` 管，⛔ 由本键）；
///   ⚠ 这是**4/4 读数**、⛔ 全局承诺：`accent` 是**格级**读数（同一格上的所有鼓件共享它），
///   4/4 的军鼓拍 0/2 在两种分组下都已达 `STRONG` 档所以不变；实测在 3/4 上给 `[1, 1, 1]`
///   会把军鼓第二击的重音从 0 变 1（缺省 `snare_acc=[1,0]` ↔ `[1,1]`）；
/// * **底鼓落点拍集合** = `{0,1,2,3}`（缺省 `{0,2}`，读数 1：`[1,1,1,1]` ⇒ 每一拍都是组起点）；
/// * **重音档条数**从 `8`/小节涨到 `14`/小节（`weight` 取两读数的较大者 ⇒ 力度档跟着变，
///   读数 2：这条**只有**在比"力度"时才看得见，比网格会漏掉它）；
/// * 和声各声部与旋律与缺省跑**逐字节**相同（D3 的消费面）。
#[test]
fn the_grouping_moves_the_kick_and_the_accents_not_the_grid() {
    let base = voices(&plan(Given::theory()));
    let moved = voices(&plan(Given::theory().grouping(Some(&EVERY_BEAT))));
    let base_drums = &base["Drums"];
    let moved_drums = &moved["Drums"];

    assert_eq!(
        base.keys().collect::<Vec<_>>(),
        moved.keys().collect::<Vec<_>>(),
        "声部集合必须不变"
    );
    for part in preset_parts(STYLE).expect("夹具必须在册") {
        assert_eq!(
            base[*part], moved[*part],
            "和声声部 `{part}` 必须与缺省跑逐条相同（本键只作用于鼓轨）"
        );
    }
    assert_eq!(
        base["Melody"], moved["Melody"],
        "旋律声部必须与缺省跑逐条相同（本键只作用于鼓轨）"
    );
    assert_eq!(
        grid_of(base_drums, HAT),
        grid_of(moved_drums, HAT),
        "踩镲的 (起点, 时值, 音高) 必须逐条不变（本键⛔ 改网格）"
    );
    assert_eq!(
        shapes_of(base_drums, SNARE),
        shapes_of(moved_drums, SNARE),
        "军鼓的 (起点, 时值, 音高, 力度) 必须逐条不变（落点由 `drumBackbeat` 管）"
    );
    assert_eq!(
        beats_of(base_drums, KICK),
        [0u64, 2].into_iter().collect::<BTreeSet<u64>>(),
        "缺省口径下 4/4 的底鼓落在拍 0 与 2（读数 1）"
    );
    assert_eq!(
        beats_of(moved_drums, KICK),
        [0u64, 1, 2, 3].into_iter().collect::<BTreeSet<u64>>(),
        "`{EVERY_BEAT:?}` ⇒ 每一拍都是组起点 ⇒ 底鼓落在每一拍（⛔ 只看条数）"
    );
    let bars = usize::try_from(BARS).expect("夹具小节数必须落进 usize");
    assert_eq!(
        accent_count(base_drums),
        8 * bars,
        "缺省口径的重音档条数（读数 2：8/小节）"
    );
    assert_eq!(
        accent_count(moved_drums),
        14 * bars,
        "`{EVERY_BEAT:?}` 的重音档条数（读数 2：14/小节 —— 组起点变多 ⇒ `weight` 变高 ⇒ 更少格子落在下档）"
    );
    assert!(
        base_drums.values().any(|note| note.3 == THEORY_TIER),
        "夹具必须真的用到下档力度（否则重音读数无从对照）"
    );
}

/// ⭐ 判据 3（D2 的形状面，4/4 夹具）：不合法的分组 ⇒ 点名本键、载荷回显取值、
/// 语义面还带 `beatsPerBar: 4`（`4` = 现读 `felt_beats_per_bar(4/4)`）、⛔ `reason` 键；
/// 合法端点 `[4]` / `[2,2]` / `[1,1,1,1]` 必须放行。
///
/// ⚠ 分层：空表、含 `0`、总和不等（`[3]` 少一拍 / `[3,2]` 多一拍 / `[5]`）都**必须**是本层的
/// 参数拒绝 —— theory 的 `BeatGrouping::new` 对它们返回 `None`（⛔ 错误），
/// 放下去只会得到兜底分支那句不点名本键的话（D6）。
#[test]
fn illegal_groupings_are_rejected_naming_the_key() {
    for raw in [
        Vec::new(),
        vec![0u64],
        vec![3],
        vec![3, 2],
        vec![5],
        vec![1, 1, 1],
    ] {
        assert_rejects_grouping(STYLE, &raw, Some(FELTS));
    }
    // 类型面：`u8` 表达不了 ⇒ 只点名，不报 `beatsPerBar`。
    assert_rejects_grouping(STYLE, &[300], None);
    for raw in [&ONE_GROUP[..], &DERIVED[..], &EVERY_BEAT[..]] {
        try_plan(Given::theory().grouping(Some(raw))).unwrap_or_else(|fault| {
            panic!("`beatGrouping` = {raw:?} 在 4/4 上合法，必须放行: {fault:?}")
        });
    }
}

/// ⭐ 判据 4（D3 的消费面，**两向都钉**）：本键要求材料源 = theory **且**鼓轨在场；
/// 反钉"⛔ 把旋律那条更严的要求扩散过来" —— `includeMelody:false` 时给本键必须放行。
#[test]
fn the_grouping_key_requires_the_drum_part_on_either_source() {
    // ⭐ 审计 `N1` 正例：`'project'` 源 + 鼓轨在场 ⇒ 本键被消费 ⇒ 放行
    // （材料源一条已废，消费口径只剩"鼓轨在场"）。
    for material_source in [None, Some("project")] {
        try_plan(
            Given::theory()
                .material_source(material_source)
                .grouping(Some(&DERIVED)),
        )
        .unwrap_or_else(|fault| {
            panic!("`beatGrouping` 在 project 源 + 鼓轨在场时必须放行 [审计 N1]: {fault:?}")
        });
    }
    let fault = try_plan(Given::theory().drums(false).grouping(Some(&DERIVED)))
        .expect_err("`beatGrouping` 不得在没有鼓轨时静默失效");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "`beatGrouping` 脱离 includeDrums: {fault:?}"
    );
    assert!(
        data(&fault)["hint"]
            .as_str()
            .is_some_and(|text| text.contains("includeDrums")),
        "载荷必须指出要补 includeDrums: {}",
        data(&fault)
    );
    try_plan(Given::theory().melody(false).grouping(Some(&DERIVED))).unwrap_or_else(|fault| {
        panic!("`beatGrouping` ⛔ 要求旋律声部在场（它与鼓线其余四键同一支口径）: {fault:?}")
    });
}

/// ⭐ 判据 5（D2/D5 的"合法形状随拍号移动"那一支）：同一条分组在 4/4 上合法、在 3/4 上被拒，
/// 且 `beatsPerBar` 跟着变成 3 —— 这是本键与 `drumBackbeat`（界是单个整数）的**结构性差别**，
/// 也是"本层⛔ 抄一张「拍号 → 拍数」表、判定必须来自 theory"的唯一防线：
/// 把总和写死成 4，这一条就会在 3/4 上红。
#[test]
fn the_legal_shape_moves_with_the_meter() {
    assert_rejects_grouping(STYLE_34, &[3, 2], Some(FELTS_34));
    assert_rejects_grouping(STYLE_34, &[4], Some(FELTS_34));
    for raw in [&[2u64, 1][..], &[3][..], &[1, 1, 1][..]] {
        try_plan(Given::theory().style(STYLE_34).grouping(Some(raw))).unwrap_or_else(|fault| {
            panic!(
                "`{STYLE_34}` 的 `beatGrouping` = {raw:?} 在 {} 拍上合法，必须放行: {fault:?}",
                FELTS_34
            )
        });
    }
    // 键缺席在两个拍号上都必须放行（缺省 = 不覆盖拍号自身的度量层级，⛔ 随拍号写死一条）。
    for style_preset in [STYLE, STYLE_34] {
        try_plan(Given::theory().style(style_preset)).unwrap_or_else(|fault| {
            panic!("`{style_preset}` 的缺省鼓轨（不给本键）必须放行: {fault:?}")
        });
    }
    // "缺省 ≡ 该拍号的复现分组"在第二个拍号上再钉一次（读数：3/4 ⇒ `[3]` 与缺省逐字节同形）
    // ⇒ 把 4/4 的 `[2, 2]` 当跨拍号常量会被这一条抓住。
    assert_eq!(
        plan(Given::theory().style(STYLE_34)),
        plan(Given::theory().style(STYLE_34).grouping(Some(&DERIVED_34))),
        "3/4 上键缺席必须逐字节等于给 `{DERIVED_34:?}`（该拍号的复现分组）"
    );
}

/// ⭐ 判据 6：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON）。
/// ⭐ 契约⛔ 登记 `default`（缺省是"不覆盖拍号自身的度量层级"）、⛔ 登记逐项 `maximum`、
/// ⛔ 登记 `maxItems`（三者的上界都是该拍号的拍数，写死会对窄拍号报出"契约说能、本层拒"的矛盾）。
/// 类型必须是 array，且 item 形状必须声明"正整数"（工具面的"非数组 / 非整数"由这道类型闸承担）。
#[test]
fn drum_grouping_is_declared_on_both_param_surfaces_without_default_or_maximum() {
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
        .param("beatGrouping")
        .expect("参数表必须声明 `beatGrouping`");
    assert_eq!(param.json_type, "array", "`beatGrouping` 必须是数组参数");
    assert!(
        !param.required,
        "缺省 = 不覆盖拍号自身的度量层级 ⇒ 必须可选"
    );
    assert!(
        param.doc.contains("ADR-0046") && param.doc.contains("拍内分组"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );
    let property = &declared["beatGrouping"];
    assert!(
        !property.is_null(),
        "契约 JSON 必须登记 `beatGrouping` 的属性: {text}"
    );
    assert_eq!(property["type"], json!("array"));
    assert_eq!(
        property["items"]["type"],
        json!("integer"),
        "item 形状必须声明整数: {property}"
    );
    assert_eq!(
        property["items"]["minimum"],
        json!(1),
        "item 下界不随拍号移动 ⇒ 必须登记 `minimum: 1`: {property}"
    );
    assert_eq!(
        property["minItems"],
        json!(1),
        "空表在任何拍号上都不合法 ⇒ 必须登记 `minItems: 1`: {property}"
    );
    assert!(
        property.get("maxItems").is_none(),
        "本键⛔ 写 `maxItems`：组数的上界 = 该拍号的拍数（4/4 是 4、6/8 是 2），\
         写一个固定数字会对窄拍号报出\"契约说能、本层拒\"的矛盾（`ADR-0046` D5）: {property}"
    );
    assert!(
        property.get("default").is_none(),
        "本键⛔ 写固定 `default`：缺省语义是\"不覆盖拍号自身的度量层级\"，而那条口径随拍号移动: {property}"
    );
    assert!(
        property["items"].get("maximum").is_none(),
        "逐项⛔ 写 `maximum`：上界 = 该拍号的拍数: {property}"
    );
}
