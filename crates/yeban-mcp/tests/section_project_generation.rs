//! `审计 N1`：`includeDrums` / `includeMelody` 放开到 `'project'` 材料源 ——
//! 鼓轨不读和声材料（流派 `GenreRule` 自带网格/音色/重音）⇒ 两源同口径；
//! 旋律需要和弦跨度 ⇒ 逐小节从和声材料片段的 voicing 判出（`material_spans`），
//! 音阶判不出来 ⇒ 要求显式 `scale`。
//!
//! 夹具材料：`filled_project` 的 MIDI 片段第一小节是 `{60,64,67,72}` =
//! Cmaj7 四音类（`Chord::from_voicing` 判得出）；后三小节无相交音符 ⇒
//! 判据 2 顺带守住"延音沿用上一小节"那条语义。

use std::collections::BTreeMap;

use serde_json::Value;
use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::music::MidiNote;
use yeban_model::{ClipContent, EntityId, Op, samples::filled_project};

const BARS: u64 = 4;
const STYLE: &str = "synthwave";
/// GM 打击乐键（底鼓 36 / 军鼓 38 / 踩镲 42 / 吊镲 51）。
const GM_DRUM_KEYS: [u8; 4] = [36, 38, 42, 51];

#[derive(Clone, Copy, Default)]
struct Given {
    material_source: Option<&'static str>,
    scale: Option<&'static str>,
    include_drums: bool,
    include_melody: bool,
    swing_permille: Option<u64>,
    melody_seed: Option<u64>,
}

fn try_plan(
    project: &yeban_model::YebanProjectV1,
    given: Given,
) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        project,
        "Chorus",
        STYLE,
        BARS,
        given.scale,
        PlanOptions {
            material_source: given.material_source,
            progression: None,
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
            drum_backbeat: None,
            drum_style: None,
            melody_seed: given.melody_seed,
            harmony_seed: None,
            material_clip_ids: None,
            part_tracks: None,
            beat_grouping: None,
        },
    )
}

fn plan(project: &yeban_model::YebanProjectV1, given: Given) -> SectionPlan {
    try_plan(project, given).unwrap_or_else(|fault| panic!("规划必须成功: {fault:?}"))
}

/// 按声部名取音符表 `(起点, 时值, 音高, 力度)`。
fn part_notes(section_plan: &SectionPlan, part: &str) -> Vec<(u64, u64, u8, u8)> {
    section_plan
        .ops
        .iter()
        .filter_map(|op| match op {
            Op::AddClip { clip } if clip.name.ends_with(part) => {
                let ClipContent::Midi { notes } = &clip.content else {
                    panic!("{part} 必须是 MIDI 片段");
                };
                Some(
                    notes
                        .values()
                        .map(|n| (n.start_tick, n.duration_ticks, n.pitch, n.velocity))
                        .collect(),
                )
            }
            _ => None,
        })
        .next()
        .unwrap_or_else(|| panic!("必须存在 `{part}` 声部的片段"))
}

fn data(fault: &BuildFault) -> &Value {
    let BuildFault::Domain { data, .. } = fault else {
        panic!("必须是领域失败: {fault:?}");
    };
    data
}

/// 判据 1：`'project'` 源上 `includeDrums` 生成 GM 鼓轨；鼓键
/// （`swingPermille`）在两源同口径被消费 ⇒ 同给时换一版网格。
#[test]
fn drums_generate_on_project_source() {
    let project = filled_project();
    let straight = plan(
        &project,
        Given {
            include_drums: true,
            ..Given::default()
        },
    );
    let hits = part_notes(&straight, "Drums");
    assert!(!hits.is_empty(), "鼓轨必须有击点");
    assert!(
        hits.iter().all(|hit| GM_DRUM_KEYS.contains(&hit.2)),
        "全部击点必须是 GM 打击乐键 {GM_DRUM_KEYS:?}: {hits:?}"
    );
    // `swingPermille` 在 project 源上同样被消费（换一版 ⇒ 网格起点变）。
    let swung = plan(
        &project,
        Given {
            include_drums: true,
            swing_permille: Some(660),
            ..Given::default()
        },
    );
    assert_ne!(
        hits.iter().map(|hit| hit.0).collect::<Vec<_>>(),
        part_notes(&swung, "Drums")
            .iter()
            .map(|hit| hit.0)
            .collect::<Vec<_>>(),
        "swing=660 必须推移鼓点起点（否则 swingPermille 没被消费）"
    );
}

/// 判据 2：`'project'` 源上 `includeMelody` —— 显式 `scale` 在场时
/// 从材料判出 Cmaj7 跨度并生成旋律；`scale` 缺省 ⇒ 响亮拒（音阶判不出）。
#[test]
fn melody_generates_over_project_material_with_explicit_scale() {
    let project = filled_project();
    let fault = try_plan(
        &project,
        Given {
            include_melody: true,
            ..Given::default()
        },
    )
    .expect_err("`project` 源上 includeMelody 缺省 `scale` 必须拒绝");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());

    let section_plan = plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: true,
            ..Given::default()
        },
    );
    let melody = part_notes(&section_plan, "Melody");
    assert!(!melody.is_empty(), "旋律必须有音符");
    assert!(
        melody.iter().all(|note| (48..=84).contains(&note.2)),
        "旋律音高必须落在缺省窗口 48..=84: {melody:?}"
    );
    // 确定性：同参数两次 ⇒ 逐条同形。
    let again = plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: true,
            ..Given::default()
        },
    );
    assert_eq!(melody, part_notes(&again, "Melody"));
}

/// 判据 3：材料片段逐小节判不出 voicing（<3 个音类）⇒ `InvalidParameterRange`
/// 带 `bar` 与 `hint`；⛔ 静默换和弦、⛔ 用邻近和弦充数。
#[test]
fn unvoiced_material_is_a_loud_failure() {
    let mut project = filled_project();
    // 把片段池里 MIDI 片段改残：第 0 小节只剩两个音类。
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            let mut sparse = BTreeMap::new();
            for (index, pitch) in [60_u8, 64].iter().enumerate() {
                let id = EntityId::new();
                sparse.insert(id, MidiNote::new(id, index as u64 * 960, *pitch, 480));
            }
            *notes = sparse;
        }
    }
    let fault = try_plan(
        &project,
        Given {
            scale: Some("C major"),
            include_melody: true,
            ..Given::default()
        },
    )
    .expect_err("判不出 voicing 的材料必须响亮拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    assert_eq!(data(&fault).get("bar"), Some(&Value::from(0)));
    assert!(data(&fault).get("reason").is_none());
    assert!(data(&fault).get("hint").is_some());
}
