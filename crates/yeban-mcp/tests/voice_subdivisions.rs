//! `ADR-0035`（审计缺口 `G8c`）：`yeban_propose_section` 的 `voiceSubdivisions`。
//!
//! 单位口径与 `ADR-0033` 完全一致：**每个 `ChordSpan` 切成几条**（⛔ 每小节击点数）。
//! 本票唯一的 new thing 是**给值的形状**：一个对象按键名把 N 分给不同声部。
//! 与姊妹文件 `voice_subdivision.rs` 一样只走领域层 `plan_with_options` —— 判据数的是
//! **生成的音符表**，把引擎拉进来只增加噪声。
//!
//! ⚠ 夹具用的两个预设的声部清单是**从 `preset_parts` 读回来的**（`synthwave` 3 声部、
//! `funk` 4 声部），本文件⛔ 手抄任何一份声部名单。

use std::collections::BTreeMap;

use serde_json::{Value, json};
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, MAX_VOICE_SUBDIVISION, PlanOptions, SectionPlan, plan_with_options,
    preset_parts,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};

/// 一条音符的可读数形状：`(身份, 起点 tick, 时值 tick, 音高, 力度)`。
type Row = (String, u64, u64, u8, u8);

/// `synthwave`（3 声部）的 theory 规划。
fn plan(uniform: Option<u64>, overrides: Option<&Value>) -> SectionPlan {
    plan_at("synthwave", uniform, overrides)
}

/// 指定预设的 theory 规划（只出和声声部：鼓/旋律不在本票作用面内）。
fn plan_at(style_preset: &str, uniform: Option<u64>, overrides: Option<&Value>) -> SectionPlan {
    plan_try(style_preset, Some("theory"), uniform, overrides).expect("theory 规划必须成功")
}

fn plan_try(
    style_preset: &str,
    material_source: Option<&str>,
    uniform: Option<u64>,
    overrides: Option<&Value>,
) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        style_preset,
        4,
        Some("C minor"),
        PlanOptions {
            material_source,
            progression: None,
            include_drums: None,
            include_melody: None,
            drum_density: None,
            voice_subdivision: uniform,
            voice_subdivisions: overrides,
            onsets_per_bar: None,
            velocity_min: None,
            velocity_max: None,
            melody_lower: None,
            melody_upper: None,
        },
    )
}

/// 按**声部名**索引的音符表（名字取 `"{section} · {part}"` 的后半）。
///
/// 表按行排序 ⇒ 两个规划可直接比对。
fn voices(plan: &SectionPlan) -> BTreeMap<String, Vec<Row>> {
    let mut map = BTreeMap::new();
    for op in &plan.ops {
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

/// 一个声部按起点排好序的 `(起点, 时值, 音高)`。
fn by_start(rows: &[Row]) -> Vec<(u64, u64, u8)> {
    let mut sorted: Vec<(u64, u64, u8)> = rows
        .iter()
        .map(|&(_, start, duration, pitch, _)| (start, duration, pitch))
        .collect();
    sorted.sort();
    sorted
}

/// `synthwave` + `theory` + 均匀旋钮缺省的**可失败**规划（拒绝类判据用它：
/// `plan` 是 `.expect()` 过的不可失败夹具，拿它的返回值调 `expect_err` 编译不过）。
fn rejected(overrides: Option<&Value>) -> Result<SectionPlan, BuildFault> {
    plan_try("synthwave", Some("theory"), None, overrides)
}

/// ⭐ 判据 1（`ADR-0035` D3）：不给该键 ≡ 空对象 ≡ `voiceSubdivision: 1`，
/// 且与 `ADR-0033` 的缺省路径**整个规划**逐字节相同（`SectionPlan: PartialEq`）。
#[test]
fn voice_subdivisions_absent_is_byte_identical_to_today() {
    let absent = plan(None, None);
    let empty = plan(None, Some(&json!({})));
    let uniform_one = plan(Some(1), None);
    assert_eq!(
        absent, empty,
        "`voiceSubdivisions: {{}}` 必须逐字节等于键缺席（没有覆盖 = 什么都不做）"
    );
    assert_eq!(
        absent, uniform_one,
        "`voiceSubdivision: 1` 必须逐字节等于键缺席（`ADR-0033` D3 的既有契约）"
    );
    // 缺省形状本身也仍是块状和声：每声部音符首尾相接、力度恒 96。
    let base = voices(&absent);
    assert_eq!(
        base.len(),
        preset_parts("synthwave").expect("synthwave 必须在册").len(),
        "夹具必须真的产出每个和声声部: {:?}",
        base.keys().collect::<Vec<_>>()
    );
    for (part, rows) in &base {
        let sorted = by_start(rows);
        for index in 1..sorted.len() {
            let (start, duration, _) = sorted[index - 1];
            assert_eq!(
                start + duration,
                sorted[index].0,
                "`{part}` 的缺省块状和声必须首尾相接: {sorted:?}"
            );
        }
        for row in rows {
            assert_eq!(
                row.4, 96,
                "`{part}` 的力度必须恒 96（本票不动力度面）: {row:?}"
            );
        }
    }
}

/// ⭐ 判据 2（D1）：`{"Bass": 4}` ⇒ **只有** bass 每跨度变 4 条，其余声部
/// 与缺省**逐字节相同**（含音符身份）。
#[test]
fn voice_subdivisions_apply_to_the_named_voice_only() {
    let base = voices(&plan(None, None));
    let got = voices(&plan(None, Some(&json!({"Bass": 4}))));
    let parts = preset_parts("synthwave").expect("synthwave 必须在册");
    assert_eq!(got.len(), base.len(), "覆盖表不得增删声部");
    for part in parts {
        let (want, rows) = (&base[*part], &got[*part]);
        if *part == "Bass" {
            assert_eq!(
                rows.len(),
                want.len() * 4,
                "`Bass` 必须被切成 4 条/跨度（{} 条 vs 缺省 {} 条）",
                rows.len(),
                want.len()
            );
        } else {
            assert_eq!(
                rows, want,
                "没被点名的 `{part}` 必须与缺省逐字节相同（含 id 种子）"
            );
        }
    }
}

/// ⭐ 判据 3（D1 的组合规则，唯一一条）：`voiceSubdivision: 4 + {"Soprano": 1}`
/// ⇒ 表里列出的用表里的值，**没列出的沿用均匀旋钮**。
#[test]
fn voice_subdivisions_compose_over_the_uniform_knob() {
    let base = voices(&plan(None, None));
    let uniform_only = voices(&plan(Some(4), None));
    let got = voices(&plan(Some(4), Some(&json!({"Soprano": 1}))));
    for part in preset_parts("synthwave").expect("synthwave 必须在册") {
        let factor = usize::try_from(if *part == "Soprano" { 1 } else { 4 }).expect("小整数");
        assert_eq!(
            got[*part].len(),
            base[*part].len() * factor,
            "`{part}` 在 `voiceSubdivision:4 + {{\"Soprano\":1}}` 下必须是 {factor}× 缺省击点数"
        );
        if *part == "Soprano" {
            assert_eq!(
                got[*part], base[*part],
                "`Soprano` 被显式压回 1 ⇒ 必须与缺省形状逐字节相同"
            );
        } else {
            assert_eq!(
                got[*part], uniform_only[*part],
                "`{part}` 没被列出 ⇒ 必须与只给 `voiceSubdivision: 4` 逐字节相同"
            );
        }
    }
}

/// ⭐ 判据 4（D1：沿用 `ADR-0033` 的算式，逐声部各算一遍）：每个被切的声部
/// 跨度时值**守恒**、相邻**无缝无叠**、音高序列与缺省逐条一致、身份互不相同。
#[test]
fn voice_subdivisions_preserve_every_span_without_overlap_per_voice() {
    let base = voices(&plan(None, None));
    let table = json!({"Bass": 8, "Alto": 3});
    let got = voices(&plan(None, Some(&table)));
    for (part, wanted_n) in [("Bass", 8u64), ("Alto", 3u64)] {
        let rows = &got[part];
        let want = &base[part];
        let n = usize::try_from(wanted_n).expect("小整数");
        assert_eq!(
            rows.len(),
            want.len() * n,
            "`{part}` 必须是 {wanted_n}× 缺省击点数"
        );
        let sum: u64 = rows.iter().map(|row| row.2).sum();
        let want_sum: u64 = want.iter().map(|row| row.2).sum();
        assert_eq!(
            sum, want_sum,
            "`{part}` 的时值合计必须守恒（最后一条吸收余数）"
        );
        let sorted = by_start(rows);
        let want_sorted = by_start(want);
        for (index, &(start, duration, pitch)) in sorted.iter().enumerate() {
            assert!(
                duration > 0,
                "`{part}` 时值必须非零: 第 {index} 条起点 {start}"
            );
            if let Some(next) = sorted.get(index + 1) {
                assert_eq!(
                    start + duration,
                    next.0,
                    "`{part}` 第 {index} 条与下一条之间必须无缝无叠: {sorted:?}"
                );
            }
            let span = want_sorted[index / n];
            assert!(
                start >= span.0 && start + duration <= span.0 + span.1,
                "`{part}` 第 {index} 条越出所属跨度 {span:?}: 起点 {start} 时值 {duration}"
            );
            assert_eq!(
                pitch, span.2,
                "`{part}` 的音高序列必须与缺省逐条一致（切分只动节奏）"
            );
        }
        let ids: BTreeMap<&String, ()> = rows.iter().map(|row| (&row.0, ())).collect();
        assert_eq!(
            ids.len(),
            rows.len(),
            "`{part}` 的音符身份必须互不相同（否则后写覆盖前写）"
        );
    }
    // 表里没点名的声部（`synthwave` 的 `Soprano`）仍是一整条满跨度。
    assert_eq!(
        got["Soprano"], base["Soprano"],
        "`Soprano` 没在表里 ⇒ 必须与缺省逐字节相同"
    );
}

/// ⭐ 判据 5（`ADR-0035` D2）：键不是该预设的声部名 ⇒ 响亮拒绝并**点名**可给的清单。
///
/// `"Drums"` 是调用方最可能误写的键（它确实是个轨，只是不由本参数管），
/// ⛔ 静默读成"没有覆盖"。
#[test]
fn voice_subdivisions_unknown_part_names_the_allowed_parts() {
    for style in ["synthwave", "funk"] {
        let fault = plan_try(style, Some("theory"), None, Some(&json!({"Drums": 2})))
            .expect_err("非声部名的键不得被静默忽略");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "未知声部名必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let BuildFault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(data["part"], json!("Drums"), "载荷必须点名是哪个键: {data}");
        let allowed = data["allowedParts"]
            .as_array()
            .unwrap_or_else(|| panic!("载荷必须给出 `allowedParts`: {data}"));
        let want = preset_parts(style).expect("预设在册");
        assert_eq!(
            allowed,
            &want.iter().map(|part| json!(part)).collect::<Vec<_>>(),
            "`allowedParts` 必须等于 `preset_parts({style})`（⛔ 两份清单）: {data}"
        );
        for absent in ["Drums", "Melody"] {
            assert!(
                !allowed.contains(&json!(absent)),
                "和声声部清单里不该出现追加声部 `{absent}`（它不归本参数管）: {data}"
            );
        }
    }
}

/// ⭐ 判据 6（D2）：`0` / 越界 / 非整数三例都必须响亮，且带 `part` 与 `range`。
#[test]
fn voice_subdivisions_out_of_range_and_non_integer_are_loud() {
    for illegal in [
        json!(0),
        json!(17),
        json!(999),
        json!(2.5),
        json!(-1),
        json!("4"),
    ] {
        let fault = rejected(Some(&json!({"Bass": illegal})))
            .expect_err("越界/非整数不得静默夹紧或四舍五入");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let BuildFault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data["part"],
            json!("Bass"),
            "载荷必须点名是哪个声部: {data}"
        );
        assert_eq!(
            data["range"],
            json!([1, MAX_VOICE_SUBDIVISION]),
            "载荷必须给出上下界: {data}"
        );
    }
    // 值不是整数**且**键也不在册时，先报哪个？—— 逐条目按"名 → 值"的顺序判，
    // 所以报的是未知名（`part` + `allowedParts`）。这条钉住实现里唯一的解释。
    let fault = rejected(Some(&json!({"Drums": 2.5}))).expect_err("未知名必须响亮");
    let BuildFault::Domain { data, .. } = &fault else {
        panic!("必须是领域失败");
    };
    assert!(
        data.get("allowedParts").is_some(),
        "未知名优先于取值错误（先告诉调用方能给什么名字）: {data}"
    );
    // 整个参数不是对象 ⇒ 同样响亮，且给出期望形状。
    for shape in [json!([8, 4]), json!("Bass=8"), json!(4), json!(null)] {
        let fault = rejected(Some(&shape)).expect_err("`voiceSubdivisions` 必须是对象");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "形状错误必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
    }
}

/// ⭐ 判据 7（D2 的耦合）：给了它却不走 `materialSource: "theory"` ⇒ 带 `hint` 拒绝。
#[test]
fn voice_subdivisions_require_theory_material_source() {
    for material_source in [None, Some("project")] {
        let fault = plan_try(
            "synthwave",
            material_source,
            None,
            Some(&json!({"Bass": 4})),
        )
        .expect_err("`voiceSubdivisions` 不得脱离 materialSource:\"theory\" 单独给");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "耦合违背必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let BuildFault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data["hint"], "请指定 materialSource: \"theory\"（逐声部切分只作用于生成的和声）",
            "必须给出补 `materialSource` 的 `hint`: {data}"
        );
    }
}

/// ⭐ 判据 8（D3 的种子规则按**声部**判定）：改一个声部⛔ 连带翻动别人的音符身份。
///
/// 这是 `ARCH-DET-001` 与"缺省兼容契约"的交点：`N == 1` 的声部必须仍走**两段式**
/// 种子 ⇒ 它的每个 id 与今天完全相同。
#[test]
fn voice_subdivisions_keep_untouched_voices_note_ids_stable() {
    let base = voices(&plan(None, None));
    let got = voices(&plan(None, Some(&json!({"Alto": 5}))));
    let parts = preset_parts("synthwave").expect("synthwave 必须在册");
    for part in parts {
        let ids: Vec<&String> = got[*part].iter().map(|row| &row.0).collect();
        let base_ids: Vec<&String> = base[*part].iter().map(|row| &row.0).collect();
        if *part == "Alto" {
            assert_ne!(
                ids, base_ids,
                "被切成 5 的声部必须换身份（种子追加切分序号，否则同跨度互相覆盖）"
            );
            assert_eq!(ids.len(), base_ids.len() * 5, "`Alto` 的条数必须是 5×");
        } else {
            assert_eq!(
                ids, base_ids,
                "`{part}` 没被点名 ⇒ 音符 id 序列必须与缺省逐字节相同"
            );
        }
    }
}

/// ⭐ 判据 9（D2 的"空对象合法"）：`{}` = 不给，且与均匀旋钮的组合也逐字节相同。
#[test]
fn voice_subdivisions_empty_object_is_the_default() {
    assert_eq!(
        plan(None, Some(&json!({}))),
        plan(None, None),
        "`{{}}` 必须等于键缺席"
    );
    for uniform in [1u64, 4, MAX_VOICE_SUBDIVISION] {
        assert_eq!(
            plan(Some(uniform), Some(&json!({}))),
            plan(Some(uniform), None),
            "`voiceSubdivision: {uniform}` + `{{}}` 必须等于只给均匀旋钮"
        );
    }
}

/// ⭐ 判据 10（`ARCH-DET-001`）：同一请求两次 ⇒ 同一份载荷；
/// 对象里键的**书写顺序**不参与身份（表先被读成有序映射）。
#[test]
fn voice_subdivisions_are_deterministic_across_two_calls() {
    let table = json!({"Soprano": 1, "Bass": 6});
    let again = json!({"Bass": 6, "Soprano": 1});
    let first = plan(None, Some(&table));
    let second = plan(None, Some(&table));
    let reordered = plan(None, Some(&again));
    assert_eq!(first, second, "同一请求两次必须得到同一份规划");
    assert_eq!(
        first, reordered,
        "JSON 对象里键的书写顺序⛔ 参与身份（`serde_json::Map` 有序 ⇒ 遍历确定）"
    );
}

/// ⭐ 判据 11（声明面同源）：`tools.rs` 的 `ParamSpec` 与契约 JSON 都必须登记这个键，
/// 且上界来自领域层的 `MAX_VOICE_SUBDIVISION`（⛔ 三份数字）。
#[test]
fn voice_subdivisions_is_declared_on_both_param_surfaces() {
    let spec = TOOLS
        .iter()
        .find(|tool| tool.name == "yeban_propose_section")
        .expect("注册表里必须有 `yeban_propose_section`");
    let param = spec
        .param("voiceSubdivisions")
        .unwrap_or_else(|| panic!("参数表必须声明 `voiceSubdivisions`"));
    assert_eq!(param.json_type, "object", "必须是对象参数");
    assert!(!param.required, "缺省 = 既有行为 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0035") && param.doc.contains("1..=16"),
        "参数说明必须带上界与出处: {}",
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
        ["properties"]["voiceSubdivisions"];
    assert!(
        !declared.is_null(),
        "契约 JSON 必须登记 `voiceSubdivisions` 的属性: {text}"
    );
    assert_eq!(declared["type"], json!("object"));
    assert_eq!(
        declared["additionalProperties"]["maximum"],
        json!(MAX_VOICE_SUBDIVISION),
        "契约上界必须等于领域层常量（⛔ 两份数字）"
    );
    assert_eq!(declared["additionalProperties"]["minimum"], json!(1));
    assert_eq!(
        declared["propertyNames"]["enum"],
        json!(["Bass", "Tenor", "Alto", "Soprano"]),
        "契约必须列出四个候选声部名（按预设取子集由 `allowedParts` 在运行时点名）"
    );
}
