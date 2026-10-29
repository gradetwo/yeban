//! 缺口补全（审计 `N2`）：`materialSource:"project"` 此前**不可指名材料** ——
//! `project_contents` 把可用材料按 `clip_pool` 顺序轮询喂给声部，调用方
//! 想让"Bass 吃琶音片段、Soprano 吃长音片段"没有任何入口。本票加
//! `materialClipIds`：ULID 数组，第 `i` 项喂第 `i` 个声部，长度必须恰等于
//! 该流派声部数，重复项允许。
//!
//! 三类失败分开报（⛔ 折叠成一条"材料不可用"）：长度不符 ⇒
//! `INVALID_PARAMETER_RANGE`；条目不存在 / 存在但不可用（非 MIDI 或零音符）
//! ⇒ `CLIP_NOT_FOUND`。给了却配 `materialSource:"theory"` ⇒
//! `INVALID_PARAMETER_RANGE`（带 `hint`，与 `progression` 同口径）。

use std::str::FromStr as _;

use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::ids::EntityId;
use yeban_model::{ClipContent, Op, samples::filled_project};

const STYLE: &str = "synthwave"; // 3 声部

/// 由夹具扩展出的工程：原样保留全部内容，再克隆一条 MIDI 片段（id 换成新值、
/// 只留第一个音符 ⇒ 音符数与原片段**不同**，指名顺序才可被读出）。
fn two_material_project() -> (yeban_model::YebanProjectV1, EntityId, EntityId) {
    let mut project = filled_project();
    let (&first_id, first_clip) = project
        .clip_pool
        .iter()
        .find(|(_, entry)| entry.content.notes().is_some())
        .expect("夹具必须有一条 MIDI 材料");
    let mut second_clip = first_clip.clone();
    let second_id = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    second_clip.id = second_id;
    second_clip.name = "named-second".to_owned();
    let ClipContent::Midi { notes } = &mut second_clip.content else {
        panic!("克隆对象必须是 MIDI 片段");
    };
    let first_note = notes.values().next().expect("原片段必须有音符").clone();
    notes.clear();
    notes.insert(first_note.id, first_note);
    project.clip_pool.insert(second_id, second_clip);
    project.validate().expect("扩展夹具必须合法");
    (project, first_id, second_id)
}

fn try_plan(
    project: &yeban_model::YebanProjectV1,
    ids: Option<Vec<EntityId>>,
) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        project,
        "Chorus",
        STYLE,
        4,
        Some("C minor"),
        PlanOptions {
            material_source: None,
            progression: None,
            include_drums: None,
            include_melody: None,
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
            melody_seed: None,
            harmony_seed: None,
            beat_grouping: None,
            material_clip_ids: ids.as_deref(),
            part_tracks: None,
        },
    )
}

/// 每个声部 `Op::AddClip` 片段里的音符数（按 `part_clip_ids` 保序）。
fn part_note_counts(section_plan: &SectionPlan) -> Vec<usize> {
    section_plan
        .ops
        .iter()
        .filter_map(|op| {
            let Op::AddClip { clip } = op else {
                return None;
            };
            let ClipContent::Midi { notes } = &clip.content else {
                return None;
            };
            Some(notes.len())
        })
        .collect()
}

/// 判据 1：指名表**真的**决定每个声部吃哪条材料 —— `[c2, c1, c2]` 的
/// 音符数轮廓是 `(c2, c1, c2)` 各自的数量，换序则随之换序（轮询做不到）。
#[test]
fn named_materials_feed_parts_in_declared_order() {
    let (project, first_id, second_id) = two_material_project();
    let first_count = project.clip_pool[&first_id]
        .content
        .notes()
        .map_or(0, |notes| notes.len());
    let section_plan =
        try_plan(&project, Some(vec![second_id, first_id, second_id])).expect("指名材料必须能规划");
    let counts = part_note_counts(&section_plan);
    assert_eq!(
        counts,
        vec![1, first_count, 1],
        "第 i 个声部必须消费第 i 条指名材料 (second=1 音, first={first_count} 音)"
    );
}

/// 判据 2：三类失败分开报 —— 长度不符带 `received`/`requiredParts`；
/// 不存在带 `clipId`/`clipPoolEntries`；存在但不是 MIDI 带 `acceptedContent`。
#[test]
fn named_material_failures_are_reported_per_class() {
    let (project, first_id, _second_id) = two_material_project();
    // 长度不符（只给 1 条而声部是 3）。
    let fault = try_plan(&project, Some(vec![first_id])).expect_err("长度不符必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["received"], 1);
    assert_eq!(data["requiredParts"], 3);

    // 不存在：一个合法但不在池里的 ULID。
    let ghost = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap();
    let fault =
        try_plan(&project, Some(vec![first_id, ghost, first_id])).expect_err("不存在的条目必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["clipId"], ghost.to_canonical_string());

    // 存在但不可用：夹具里的 audio 片段（不是 MIDI）。
    let audio_id = project
        .clip_pool
        .iter()
        .find(|(_, entry)| entry.content.notes().is_none())
        .map(|(id, _)| *id)
        .expect("夹具必须有一条非 MIDI 片段");
    let fault = try_plan(&project, Some(vec![first_id, audio_id, first_id]))
        .expect_err("非 MIDI 条目必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::ClipNotFound));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["clipId"], audio_id.to_canonical_string());
    assert!(data["acceptedContent"].is_string());
}

/// 判据 3：theory 路径不读片段池 —— `materialClipIds` 配
/// `materialSource:"theory"` 必须响亮拒（带 `hint`）。
#[test]
fn named_materials_with_theory_source_are_rejected() {
    let (project, first_id, second_id) = two_material_project();
    let options_ids = Some(vec![first_id, second_id, first_id]);
    let fault = plan_with_options(
        &project,
        "Chorus",
        STYLE,
        4,
        Some("C minor"),
        PlanOptions {
            material_source: Some("theory"),
            progression: None,
            include_drums: None,
            include_melody: None,
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
            melody_seed: None,
            harmony_seed: None,
            beat_grouping: None,
            material_clip_ids: options_ids.as_deref(),
            part_tracks: None,
        },
    )
    .expect_err("theory 路径配指名材料必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert!(data["hint"].is_string());
}
