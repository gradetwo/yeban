//! `ADR-0036`（审计缺口 `G8d`）：`yeban_propose_section` 的 `onsetsPerBar` 与
//! "缺省网格从拍号导出"。
//!
//! 单位口径 = **每小节**音头数（⛔ `ADR-0033` / `ADR-0035` 那两个参数的"每个和弦
//! 跨度"，两者互不换算）。只走领域层 `plan_with_options` —— 判据数的是**生成的音符表**。
//!
//! ⚠ 可观察量的选定（本票草案写"每小节音头数 = `cells_per_bar`"，那对**鼓**不成立：
//! 一格可以落多个鼓（底鼓 + 踩镲同点），击点数 ≠ 网格容量。真正可数的是
//! **旋律**：`Melody::len` 恒等于网格的 onset 数（`yeban-theory` 自己的文档口径），
//! 而 `rhythm_grid(bars, onsets)` 的 onset 数 = `bars × onsets` ⇒
//! **旋律音数 = 小节数 × 每小节格位数**。本文件用这条算式量"参数真的落到了生成器"。
//!
//! ⚠ 非 4/4 预设的**名单从 `GenreLibrary::all()` 现读**（`⛔ 手抄 ["waltz", ...]`）：
//! `G18b` 那条"守卫的语义不能靠手抄名单"的同族错误 —— 流派表一改就该测不到。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options, preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};
use yeban_theory::{cells_per_bar, genre::GenreLibrary};

/// 夹具的小节数（判据里所有"音数 = 小节数 × 格位数"的算式都用它，⛔ 两处写死）。
const BARS: u64 = 4;

/// 一条音符的可读数形状：`(身份, 起点 tick, 时值 tick, 音高, 力度)`。
type Row = (String, u64, u64, u8, u8);

/// 一次请求的可给项。
///
/// ⛔ 每个判据手抄一遍 `PlanOptions` 的八个字段：那张表就是本文件的"缺省 + 一处改动"，
/// 新参数默认**已给**鼓与旋律两条消费路径（`ADR-0036` D4 的正例形状）。
#[derive(Clone, Copy)]
struct Given<'a> {
    style_preset: &'a str,
    material_source: Option<&'a str>,
    onsets_per_bar: Option<u64>,
    include_drums: bool,
    include_melody: bool,
    voice_subdivisions: Option<&'a Value>,
}

impl<'a> Given<'a> {
    /// `materialSource: "theory"` + 鼓 + 旋律，其余缺省。
    fn theory(style_preset: &'a str) -> Self {
        Self {
            style_preset,
            material_source: Some("theory"),
            onsets_per_bar: None,
            include_drums: true,
            include_melody: true,
            voice_subdivisions: None,
        }
    }

    fn onsets(mut self, value: Option<u64>) -> Self {
        self.onsets_per_bar = value;
        self
    }

    fn material_source(mut self, value: Option<&'a str>) -> Self {
        self.material_source = value;
        self
    }

    /// 只留一条消费路径（D4 的正/反例都靠它）。
    fn consumers(mut self, drums: bool, melody: bool) -> Self {
        self.include_drums = drums;
        self.include_melody = melody;
        self
    }

    fn subdivisions(mut self, overrides: Option<&'a Value>) -> Self {
        self.voice_subdivisions = overrides;
        self
    }
}

fn try_plan(given: Given<'_>) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        given.style_preset,
        BARS,
        Some("C minor"),
        PlanOptions {
            material_source: given.material_source,
            progression: None,
            include_drums: Some(given.include_drums),
            include_melody: Some(given.include_melody),
            drum_density: None,
            voice_subdivision: None,
            voice_subdivisions: given.voice_subdivisions,
            onsets_per_bar: given.onsets_per_bar,
            velocity_min: None,
            velocity_max: None,
            melody_lower: None,
            melody_upper: None,
            melody_max_leap: None,
        },
    )
}

fn plan(given: Given<'_>) -> SectionPlan {
    try_plan(given).unwrap_or_else(|fault| {
        panic!(
            "`{}` 的 theory 规划（onsets={:?}）必须成功: {fault:?}",
            given.style_preset, given.onsets_per_bar
        )
    })
}

/// 该预设拍号一小节的格位数：**缺省值与上界的同一个来源**（⛔ 手抄数字）。
fn cells(style_preset: &str) -> u64 {
    let rule =
        GenreLibrary::get(style_preset).unwrap_or_else(|_| panic!("`{style_preset}` 必须在册"));
    cells_per_bar(rule.meter_value()).unwrap_or_else(|| panic!("`{style_preset}` 的拍号必须合法"))
}

/// 按**声部名**索引的音符表（名字取 `"{section} · {part}"` 的后半）。
fn voices(section_plan: &SectionPlan) -> BTreeMap<String, Vec<Row>> {
    let mut map = BTreeMap::new();
    for op in &section_plan.ops {
        let Op::AddClip { clip } = op else { continue };
        let ClipContent::Midi { notes } = &clip.content else {
            continue;
        };
        let mut rows: Vec<Row> = notes
            .values()
            .map(|note| {
                (
                    note.id.to_canonical_string(),
                    note.start_tick,
                    note.duration_ticks,
                    note.pitch,
                    note.velocity,
                )
            })
            .collect();
        rows.sort();
        let part = clip
            .name
            .rsplit(" · ")
            .next()
            .unwrap_or(clip.name.as_str())
            .to_owned();
        map.insert(part, rows);
    }
    map
}

/// 失败载荷的 `data`（拒绝类判据都要读它）。
fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// ⭐ 判据 1（`ADR-0036` D1）：4/4 预设上"不给"≡"给 16"≡"给 `cells_per_bar(4/4)`"，
/// 整个规划逐字节相同（`SectionPlan: PartialEq`），且旋律音数 = `BARS × cells`。
#[test]
fn common_time_default_grid_is_byte_identical_to_explicit_sixteen() {
    let style = "synthwave";
    assert_eq!(
        cells(style),
        16,
        "夹具必须是 4/4（否则本条判据测不到不变量）"
    );
    let absent = plan(Given::theory(style));
    let explicit = plan(Given::theory(style).onsets(Some(16)));
    assert_eq!(
        absent, explicit,
        "4/4 上 `onsetsPerBar` 缺席必须逐字节等于给 16（`ADR-0016` 的缺省兼容契约）"
    );
    let table = voices(&absent);
    assert!(
        table.contains_key("Drums") && table.contains_key("Melody"),
        "夹具必须真的产出鼓与旋律两条消费路径: {:?}",
        table.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        table["Melody"].len(),
        usize::try_from(BARS).expect("BARS 进 usize") * 16,
        "旋律音数必须等于 `小节数 × 每小节格位数`（网格真的落到了生成器）"
    );
}

/// ⭐ 判据 2（D1 的另一半）：**每条**非 4/4 流派现在都生成得出来，且
/// "不给" ≡ "给它自己的格位数"；同时 `onsetsPerBar: 16`（= 旧常量）在这条流派上
/// 必须**响亮失败** —— 那就是 `ADR-0017` 的常量在 28 条流派上的既有缺陷。
#[test]
fn every_non_common_time_genre_generates_at_its_own_grid() {
    let mut checked: Vec<String> = Vec::new();
    for rule in GenreLibrary::all() {
        if rule.meter == (4, 4) {
            continue;
        }
        let Some(meter_cells) = cells_per_bar(rule.meter_value()) else {
            continue; // 病态拍号由 theory 的构造器拒绝，不在本票作用面内
        };
        let style = rule.id;
        checked.push(style.to_owned());
        let meter_text = format!("{:?}", rule.meter);
        let absent = plan(Given::theory(style));
        let explicit = plan(Given::theory(style).onsets(Some(meter_cells)));
        assert_eq!(
            absent, explicit,
            "流派 `{style}`（{meter_text}）：缺省网格必须等于 `cells_per_bar` = {meter_cells}"
        );
        assert_eq!(
            voices(&absent)["Melody"].len(),
            usize::try_from(BARS).expect("BARS 进 usize")
                * usize::try_from(meter_cells).expect("格位数进 usize"),
            "流派 `{style}`：旋律音数必须是 `小节数 × 格位数`"
        );
        if meter_cells >= 16 {
            continue; // 5/4（20 格）那类拍号下旧常量本来就合法，只是铺不满
        }
        let fault = try_plan(Given::theory(style).onsets(Some(16)))
            .expect_err("旧常量 16 在格位数更小的拍号上必须失败（这条钉住缺陷本身）");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "流派 `{style}` 的过密请求必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        assert_eq!(
            data(&fault)["maxOnsetsPerBar"],
            json!(meter_cells),
            "流派 `{style}`：上界必须从 theory 读回: {}",
            data(&fault)
        );
    }
    assert!(
        !checked.is_empty(),
        "非 4/4 流派必须真的被覆盖到（空循环 = 空判据）: {checked:?}"
    );
}

/// ⭐ 判据 3（D3）：过密时载荷点名 `onsetsPerBar` / `maxOnsetsPerBar` / `meter`，
/// 且 `maxOnsetsPerBar` **等于同一个表达式** `cells_per_bar(meter)`（⛔ 两份数字）。
#[test]
fn dense_request_reports_the_boundary_from_theory() {
    for style in ["synthwave", "waltz"] {
        let meter_cells = cells(style);
        let too_dense = meter_cells + 1;
        let fault = try_plan(Given::theory(style).onsets(Some(too_dense)))
            .expect_err("超过格位数必须响亮拒绝（⛔ 静默夹紧）");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "`{style}` 的过密请求必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let payload = data(&fault);
        assert_eq!(
            payload["maxOnsetsPerBar"],
            json!(meter_cells),
            "`{style}`：上界必须 = `cells_per_bar(meter)`，⛔ 本层第二张表: {payload}"
        );
        assert_eq!(
            payload["onsetsPerBar"],
            json!(too_dense),
            "载荷必须回显被拒的取值: {payload}"
        );
        let rule = GenreLibrary::get(style).expect("在册");
        assert_eq!(
            payload["meter"],
            json!(format!("{}/{}", rule.meter.0, rule.meter.1)),
            "载荷的拍号必须与该流派一致: {payload}"
        );
        assert!(
            payload["hint"]
                .as_str()
                .is_some_and(|text| text.contains("格位数")),
            "载荷必须给出可执行的修正方向: {payload}"
        );
    }
}

/// ⭐ 判据 4（D2 的形状面）：`0` 与"超出本层可表达的宽度"⇒ 带 `range` 的拒绝。
///
/// ⚠ 负数 / 非整数 / 字符串在**工具面**就被拦下（`arg_u64` 的"必须是非负整数"、
/// `ParamSpec::accepts("integer")`），纯领域层只能量到这两行 ⇒ 判据按分层如实分写。
#[test]
fn zero_and_over_width_are_shape_rejections() {
    for shape in [0, u64::from(u32::MAX) + 1] {
        let fault = try_plan(Given::theory("synthwave").onsets(Some(shape)))
            .expect_err("`0` 与超宽取值都不得被静默当作\"没给\"");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "形状错误必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        assert_eq!(
            data(&fault)["range"],
            json!([1, u32::MAX]),
            "载荷必须给出本层的下界与可表达宽度: {}",
            data(&fault)
        );
    }
}

/// ⭐ 判据 5（D2 的作用域）：网格只动鼓与旋律 —— 和声声部逐字节不变
/// （`ADR-0033` 判据 4 的对称形式），且两条消费路径**都**真的变了。
#[test]
fn grid_moves_only_drums_and_melody() {
    let style = "synthwave";
    let fine = plan(Given::theory(style).onsets(Some(16)));
    let coarse = plan(Given::theory(style).onsets(Some(8)));
    let fine_voices = voices(&fine);
    let coarse_voices = voices(&coarse);
    for part in preset_parts(style).expect("synthwave 必须在册") {
        assert_eq!(
            fine_voices[*part], coarse_voices[*part],
            "和声声部 `{part}` 必须⛔ 受每小节网格影响"
        );
    }
    assert!(
        coarse_voices["Drums"].len() < fine_voices["Drums"].len(),
        "鼓必须真的读到了网格（8 格的击点数应少于 16 格）: {} vs {}",
        coarse_voices["Drums"].len(),
        fine_voices["Drums"].len()
    );
    assert_eq!(
        coarse_voices["Melody"].len(),
        usize::try_from(BARS).expect("BARS 进 usize") * 8,
        "旋律音数必须 = `小节数 × 8`（参数落到生成器的直接读数）"
    );
}

/// ⭐ 判据 6（D4）：给了必须被消费 —— 两条耦合各一例拒绝，并各留一条正例
/// （本参数**两条消费路径任一在场**即可，⛔ 套用 `drumDensity` 那条更严的"必须有鼓"）。
#[test]
fn onsets_per_bar_requires_theory_source_and_one_consumer() {
    for material_source in [None, Some("project")] {
        let fault = try_plan(
            Given::theory("synthwave")
                .material_source(material_source)
                .onsets(Some(8))
                .consumers(false, false),
        )
        .expect_err("`onsetsPerBar` 不得脱离 materialSource:\"theory\" 单独给");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "耦合违背必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        assert!(
            data(&fault)["hint"]
                .as_str()
                .is_some_and(|text| text.contains("materialSource")),
            "载荷必须指出要补 materialSource: {}",
            data(&fault)
        );
    }
    let fault = try_plan(
        Given::theory("synthwave")
            .onsets(Some(8))
            .consumers(false, false),
    )
    .expect_err("两条消费路径都不在场时，参数无处落地");
    assert_eq!(
        fault.domain_code(),
        Some(BuildCode::InvalidParameterRange),
        "无消费方必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
    );
    // 正例：只给旋律（`drumDensity` 在这种组合下会被拒，本参数不会）。
    let melody_only = plan(
        Given::theory("synthwave")
            .onsets(Some(8))
            .consumers(false, true),
    );
    assert_eq!(
        voices(&melody_only)["Melody"].len(),
        usize::try_from(BARS).expect("BARS 进 usize") * 8,
        "旋律单独在场时 `onsetsPerBar` 必须生效",
    );
    let drums_only = plan(
        Given::theory("synthwave")
            .onsets(Some(8))
            .consumers(true, false),
    );
    assert!(
        voices(&drums_only).contains_key("Drums"),
        "鼓单独在场时规划必须成功（鼓轨是消费方之一）"
    );
}

/// ⭐ 判据 7（与 `ADR-0035` 正交）：两个参数同时给时**各自**保持自己的语义，
/// 互不折算、互不顶掉。
#[test]
fn grid_and_per_voice_subdivision_are_orthogonal() {
    let style = "synthwave";
    let overrides = json!({"Bass": 4});
    let both = plan(
        Given::theory(style)
            .onsets(Some(8))
            .subdivisions(Some(&overrides)),
    );
    let only_subdivided = plan(Given::theory(style).subdivisions(Some(&overrides)));
    let only_grid = plan(Given::theory(style).onsets(Some(8)));
    let both_voices = voices(&both);
    assert_eq!(
        both_voices["Bass"],
        voices(&only_subdivided)["Bass"],
        "`onsetsPerBar` 不得改变逐声部切分的结果"
    );
    assert_eq!(
        both_voices["Melody"],
        voices(&only_grid)["Melody"],
        "逐声部切分不得改变每小节网格的结果"
    );
    assert_eq!(
        both_voices["Drums"],
        voices(&only_grid)["Drums"],
        "逐声部切分不得改变鼓的生成"
    );
}

/// ⭐ 判据 8：两个声明面同源（注册表 `ParamSpec` ⇄ 契约 JSON）。
///
/// ⚠ 契约侧**刻意没有 `maximum`**：上界是拍号的函数（D3），写成常量就会与 theory
/// 分成两份数字。这里钉住"没有 maximum"，正是钉住 D3 那条决定。
#[test]
fn both_declaration_surfaces_name_the_parameter() {
    let spec = TOOLS
        .iter()
        .find(|tool| tool.name == "yeban_propose_section")
        .expect("注册表里必须有 `yeban_propose_section`");
    let param = spec
        .param("onsetsPerBar")
        .unwrap_or_else(|| panic!("参数表必须声明 `onsetsPerBar`"));
    assert_eq!(param.json_type, "integer", "必须是整数参数");
    assert!(!param.required, "缺省 = 从拍号导出 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0036") && param.doc.contains("每小节"),
        "参数说明必须写清单位与出处: {}",
        param.doc
    );

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("仓库根目录");
    let text = std::fs::read_to_string(repo.join("schemas").join("mcp-tools.schema.json"))
        .expect("读契约 schema");
    let contract: Value = serde_json::from_str(&text).expect("契约必须是合法 JSON");
    let declared = &contract["definitions"]["DocumentedToolArguments"]["$defs"]["yeban_propose_section"]
        ["properties"]["onsetsPerBar"];
    assert!(
        !declared.is_null(),
        "契约 JSON 必须登记 `onsetsPerBar` 的属性: {text}"
    );
    assert_eq!(declared["type"], json!("integer"));
    assert_eq!(
        declared["minimum"],
        json!(1),
        "契约下界必须与领域层的形状拒绝一致（⛔ 两份数字）"
    );
    assert!(
        declared["maximum"].is_null(),
        "`ADR-0036` D3：上界来自 theory 的 `slots`，契约里⛔ 写常量: {declared}"
    );
}
