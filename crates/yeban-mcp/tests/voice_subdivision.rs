//! `ADR-0033`（审计缺口 `G8b`）：`yeban_propose_section` 的 `voiceSubdivision`。
//!
//! 单位口径（⛔ 两种读数混用）：**每个 `ChordSpan` 切成几条**，不是每小节击点数。
//! 本文件只走 `materialSource: "theory"` 的领域层（`plan_with_options`）——
//! 判据要数的是**生成的音符表**，把引擎拉进来只会增加噪声；参数声明面
//! （`tools.rs` 的 `ParamSpec` 与契约 JSON）另在最后一个判据里核对同源。

use std::collections::BTreeMap;

use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, MAX_VOICE_SUBDIVISION, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_mcp::tools::TOOLS;
use yeban_model::{ClipContent, Op, samples::filled_project};

/// 一条音符的可读数形状：`(身份, 起点 tick, 时值 tick, 音高, 力度)`。
type Row = (String, u64, u64, u8, u8);

fn theory(voice_subdivision: Option<u64>, include_parts: bool) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        &filled_project(),
        "Chorus",
        "synthwave",
        4,
        Some("C minor"),
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: include_parts.then_some(true),
            include_melody: include_parts.then_some(true),
            drum_density: None,
            voice_subdivision,
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
            melody_seed: None,
            harmony_seed: None,
            material_clip_ids: None,
            part_tracks: None,
            beat_grouping: None,
        },
    )
}

/// 只有和声声部的缺省夹具（键缺席）。
fn theory_plan(voice_subdivision: Option<u64>) -> SectionPlan {
    theory(voice_subdivision, false).expect("theory 规划必须成功")
}

/// 和声 + 鼓 + 旋律的夹具（判据 4 用）。
fn theory_with_parts(voice_subdivision: Option<u64>) -> SectionPlan {
    theory(voice_subdivision, true).expect("theory + 鼓 + 旋律规划必须成功")
}

/// 按 `parts` 顺序取出每个 MIDI 片段的名字与音符表（表按行排序 ⇒ 可比对）。
fn clips(plan: &SectionPlan) -> Vec<(String, Vec<Row>)> {
    plan.ops
        .iter()
        .filter_map(|op| match op {
            Op::AddClip { clip } => match &clip.content {
                ClipContent::Midi { notes } => {
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
                    Some((clip.name.clone(), rows))
                }
                ClipContent::Audio { .. } => None,
            },
            _ => None,
        })
        .collect()
}

fn is_appended_part(name: &str) -> bool {
    name.ends_with("· Drums") || name.ends_with("· Melody")
}

/// 和声声部（⛔ 排除 `ADR-0017` 追加的鼓 / 旋律 —— 本票不碰它们）。
fn harmony(clips: &[(String, Vec<Row>)]) -> Vec<Vec<Row>> {
    clips
        .iter()
        .filter(|(name, _)| !is_appended_part(name))
        .map(|(_, rows)| rows.clone())
        .collect()
}

/// 一个声部里按起点排好序的音符（跨度序 = 起点序，每跨度 `N` 条连续）。
fn by_start(rows: &[Row]) -> Vec<(u64, u64, u8)> {
    let mut sorted: Vec<(u64, u64, u8)> = rows
        .iter()
        .map(|&(_, start, duration, pitch, _)| (start, duration, pitch))
        .collect();
    sorted.sort();
    sorted
}

/// ⭐ 判据 1（`ADR-0033` D3）：键缺席 ≡ `voiceSubdivision: 1`，且缺省形状仍是
/// 块状和声 —— 每声部音符首尾相接铺满整段（无缝）、力度恒 `96`。
#[test]
fn voice_subdivision_default_is_byte_identical_to_absent() {
    let absent = clips(&theory_plan(None));
    let explicit = clips(&theory_plan(Some(1)));
    assert_eq!(
        absent, explicit,
        "`voiceSubdivision: 1` 必须逐字节等于键缺席（含 id 种子**不**追加 `:0` 后缀）"
    );
    let voices = harmony(&absent);
    assert!(
        !voices.is_empty(),
        "theory 路径必须产出和声声部: {absent:?}"
    );
    for rows in &voices {
        let sorted = by_start(rows);
        for index in 1..sorted.len() {
            let (start, duration, _) = sorted[index - 1];
            assert_eq!(
                start + duration,
                sorted[index].0,
                "缺省的块状和声必须首尾相接铺满整段: {sorted:?}"
            );
        }
        for row in rows {
            assert_eq!(row.4, 96, "缺省力度必须恒 `THEORY_VELOCITY` = 96: {row:?}");
        }
    }
}

/// ⭐ 判据 2（D1）：`N ∈ {{2,3,4,6,7,16}}` 时每声部恰好 `N ×` 缺省击点数、
/// 跨度时值**守恒**、相邻音符**无缝无叠**、音高与缺省逐条相同、身份互不相同。
#[test]
fn voice_subdivision_partitions_the_span_without_loss() {
    let base = harmony(&clips(&theory_plan(None)));
    for subdivision in [2u64, 3, 4, 6, 7, MAX_VOICE_SUBDIVISION] {
        let count = usize::try_from(subdivision).expect("N 在 usize 范围内");
        let got = harmony(&clips(&theory_plan(Some(subdivision))));
        assert_eq!(got.len(), base.len(), "`N={subdivision}` 不得增删声部");
        for (want, rows) in base.iter().zip(&got) {
            assert_eq!(
                rows.len(),
                want.len() * count,
                "`N={subdivision}` 的击点数必须是 N × 缺省击点数"
            );
            let sum: u64 = rows.iter().map(|row| row.2).sum();
            let want_sum: u64 = want.iter().map(|row| row.2).sum();
            assert_eq!(sum, want_sum, "`N={subdivision}` 的时值合计必须守恒");

            let sorted = by_start(rows);
            let want_sorted = by_start(want);
            for (index, &(start, duration, pitch)) in sorted.iter().enumerate() {
                assert!(
                    duration > 0,
                    "时值必须非零（`MidiNote` 不变量）: 第 {index} 条起点 {start}"
                );
                if let Some(next) = sorted.get(index + 1) {
                    assert_eq!(
                        start + duration,
                        next.0,
                        "`N={subdivision}` 相邻音符必须无缝无叠: {sorted:?}"
                    );
                }
                // 每条都落在它所属的缺省跨度里，且音高 = 该跨度在缺省路径上的音高。
                let span = want_sorted[index / count];
                assert!(
                    start >= span.0 && start + duration <= span.0 + span.1,
                    "`N={subdivision}` 第 {index} 条越出所属跨度 {span:?}: 起点 {start} 时值 {duration}"
                );
                assert_eq!(
                    pitch, span.2,
                    "`N={subdivision}` 的音高序列必须与缺省逐条一致（切分只动节奏）"
                );
            }
            // 身份唯一性：切分序号进了种子 ⇒ 同一跨度内 `N` 条各有 id。
            let ids: BTreeMap<&String, ()> = rows.iter().map(|row| (&row.0, ())).collect();
            assert_eq!(
                ids.len(),
                rows.len(),
                "`N={subdivision}` 的音符身份必须互不相同（否则后写覆盖前写）"
            );
        }
    }
}

/// ⭐ 判据 3（D3）：越界与"给了却不走 theory"都必须响亮拒绝，同一错误码。
#[test]
fn voice_subdivision_bounds_and_coupling_are_loud() {
    for illegal in [0u64, MAX_VOICE_SUBDIVISION + 1, 999] {
        let fault = theory(Some(illegal), false).expect_err("`voiceSubdivision` 越界不得静默夹紧");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "越界必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let BuildFault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data["range"],
            serde_json::json!([1, MAX_VOICE_SUBDIVISION]),
            "错误载荷必须给出上下界: {data}"
        );
        assert_eq!(
            data["voiceSubdivision"],
            serde_json::json!(illegal),
            "错误载荷必须回显实际值: {data}"
        );
    }

    // 给了却不消费（工程材料路径没有生成的和声）⇒ 同码 + `hint`。
    for material_source in [None, Some("project")] {
        let fault = plan_with_options(
            &filled_project(),
            "Chorus",
            "synthwave",
            4,
            None,
            PlanOptions {
                material_source,
                progression: None,
                include_drums: None,
                include_melody: None,
                drum_density: None,
                voice_subdivision: Some(4),
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
                melody_seed: None,
                harmony_seed: None,
                material_clip_ids: None,
                part_tracks: None,
                beat_grouping: None,
            },
        )
        .expect_err("`voiceSubdivision` 不得脱离 materialSource:\"theory\" 单独给");
        assert_eq!(
            fault.domain_code(),
            Some(BuildCode::InvalidParameterRange),
            "耦合违背必须是 `INVALID_PARAMETER_RANGE`: {fault:?}"
        );
        let BuildFault::Domain { data, .. } = &fault else {
            panic!("必须是领域失败");
        };
        assert_eq!(
            data["hint"], "请指定 materialSource: \"theory\"（切分只作用于生成的和声）",
            "必须给出补 `materialSource` 的 `hint`: {data}"
        );
    }
}

/// ⭐ 判据 4（D1 的作用域）：切分**只**动和声，鼓与旋律逐字节不变。
#[test]
fn voice_subdivision_does_not_touch_drums_or_melody() {
    let base = clips(&theory_with_parts(None));
    let subdivided = clips(&theory_with_parts(Some(8)));
    for suffix in ["· Drums", "· Melody"] {
        assert!(
            base.iter().any(|(name, _)| name.ends_with(suffix)),
            "夹具必须真的带上 `{suffix}` 声部: {base:?}"
        );
    }
    for (name, rows) in &base {
        let counterpart = subdivided
            .iter()
            .find(|(other, _)| other == name)
            .unwrap_or_else(|| panic!("声部必须同名出现在两次规划里: {name}"));
        if is_appended_part(name) {
            assert_eq!(
                rows, &counterpart.1,
                "`voiceSubdivision` 不得改动 `{name}`（它只作用于和声声部）"
            );
        } else {
            assert_eq!(
                counterpart.1.len(),
                rows.len() * 8,
                "`{name}` 必须被切成 8 条/跨度"
            );
        }
    }
}

/// ⭐ 判据 5（声明面同源）：`tools.rs` 的参数行与契约 JSON 必须都登记这个键，
/// 且两条上界都来自领域层的 `MAX_VOICE_SUBDIVISION`（⛔ 三份数字）。
#[test]
fn voice_subdivision_is_declared_on_both_param_surfaces() {
    let spec = TOOLS
        .iter()
        .find(|tool| tool.name == "yeban_propose_section")
        .expect("注册表里必须有 `yeban_propose_section`");
    let param = spec
        .param("voiceSubdivision")
        .unwrap_or_else(|| panic!("参数表必须声明 `voiceSubdivision`"));
    assert_eq!(param.json_type, "integer", "必须是整数参数");
    assert!(!param.required, "缺省 = 既有行为 ⇒ 必须可选");
    assert!(
        param.doc.contains("ADR-0033") && param.doc.contains("1..=16"),
        "参数说明必须带上界与出处: {}",
        param.doc
    );

    let repo = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|crates| crates.parent())
        .expect("仓库根目录");
    let text = std::fs::read_to_string(repo.join("schemas").join("mcp-tools.schema.json"))
        .expect("读契约 schema");
    let contract: serde_json::Value = serde_json::from_str(&text).expect("契约必须是合法 JSON");
    let declared = &contract["definitions"]["DocumentedToolArguments"]["$defs"]["yeban_propose_section"]
        ["properties"]["voiceSubdivision"];
    assert!(
        !declared.is_null(),
        "契约 JSON 必须登记 `voiceSubdivision` 的属性: {text}"
    );
    assert_eq!(declared["type"], serde_json::json!("integer"));
    assert_eq!(declared["minimum"], serde_json::json!(1));
    assert_eq!(
        declared["maximum"],
        serde_json::json!(MAX_VOICE_SUBDIVISION),
        "契约上界必须等于领域层常量（⛔ 两份数字）"
    );
    assert_eq!(declared["default"], serde_json::json!(1));
}
