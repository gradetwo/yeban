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
//!
//! ⭐ 判据 4（2026-10-30 补）把本片从**耦合面**扩到**效果面**：`materialSource` 此前
//! 全库只有"哪些取值⛔ 能同时给"的证据，没有一条判据把同一请求在两个 source 下各跑一次
//! 并断言产出真的变。那条缺口是效果面登记表读出的两个缺口之一，现由判据 4 闭合。

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
    material_source: Option<&str>,
    ids: Option<Vec<EntityId>>,
) -> Result<SectionPlan, BuildFault> {
    plan_with_options(
        project,
        "Chorus",
        STYLE,
        4,
        Some("C minor"),
        PlanOptions {
            material_source,
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
    let section_plan = try_plan(&project, None, Some(vec![second_id, first_id, second_id]))
        .expect("指名材料必须能规划");
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
    let fault = try_plan(&project, None, Some(vec![first_id])).expect_err("长度不符必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["received"], 1);
    assert_eq!(data["requiredParts"], 3);

    // 不存在：一个合法但不在池里的 ULID。
    let ghost = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap();
    let fault = try_plan(&project, None, Some(vec![first_id, ghost, first_id]))
        .expect_err("不存在的条目必须拒");
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
    let fault = try_plan(&project, None, Some(vec![first_id, audio_id, first_id]))
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
    let fault = try_plan(
        &project,
        Some("theory"),
        Some(vec![first_id, second_id, first_id]),
    )
    .expect_err("theory 路径配指名材料必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert!(data["hint"].is_string());
}

/// 取规划成功的那一支（判据 4 的三处都用它；失败时把来源与故障一起打进 panic 文本）。
fn plan_ok(project: &yeban_model::YebanProjectV1, material_source: Option<&str>) -> SectionPlan {
    try_plan(project, material_source, None)
        .unwrap_or_else(|fault| panic!("来源 {material_source:?} 的规划必须成功: {fault:?}"))
}

/// ⭐ 判据 4：`materialSource` 的**效果面**（前三条只钉了**耦合**——哪些取值⛔ 能同时给；
/// 本条钉"换了取值，产出真的换"。这一侧此前全库无人钉，是效果面登记表读出的两个缺口之一）。
///
/// 1. **缺省钉**：不给 ≡ 给 `"project"`，整份 `SectionPlan` 逐字节相同（`ADR-0016` D2 的向后兼容）；
/// 2. **换来源即换计划**：同一请求改给 `"theory"` ⇒ 计划不同，且**段落身份**随之换
///    （`section_id` 的种子串带调式键：`'project'` 是 `Scale::canonical()`，
///    `'theory'` 是 `"<音阶>:theory:<走向>"`，见 `section_build.rs` 的 `TheoryInput::scale_key`）；
/// 3. **两条路径读的是两份材料**：`'theory'` 分支⛔ 调 `project_contents` ⇒ 片段池多一条材料
///    时 theory 计划**一位不变**；`'project'` 路径的每个和声声部都来自池条目
///    （`transposed_notes` 与源片段 1:1，转调不改音符数）⇒ 声部音符数必然落在池的读数集合里。
///    "池敏感"的另一半（指名换序 ⇒ 轮廓随之换序）已由判据 1 钉住，此处⛔ 再抄一遍。
#[test]
fn material_source_switches_the_material_and_theory_ignores_the_clip_pool() {
    let (two_material, _first_id, _second_id) = two_material_project();
    let base = filled_project();
    assert!(
        two_material.clip_pool.len() > base.clip_pool.len(),
        "夹具必须比原池多一条材料（第 3 断的敏感性全押在这个差上）: 夹具 {} / 原池 {}",
        two_material.clip_pool.len(),
        base.clip_pool.len()
    );

    let absent = plan_ok(&two_material, None);
    let explicit_project = plan_ok(&two_material, Some("project"));
    assert_eq!(
        absent, explicit_project,
        "`materialSource` 缺席必须逐字节等于给 `\"project\"`（`ADR-0016` D2 的缺省兼容契约）"
    );

    let theory = plan_ok(&two_material, Some("theory"));
    assert_ne!(
        absent, theory,
        "换了材料来源而整份计划一位没变 ⇒ 这个键只是被接受的字符串，⛔ 没有效果面"
    );
    assert_ne!(
        absent.section_id, theory.section_id,
        "段落身份的种子里带着调式键 ⇒ 两个来源的身份⛔ 相同"
    );

    let theory_on_base = plan_ok(&base, Some("theory"));
    assert_eq!(
        theory, theory_on_base,
        "theory 路径⛔ 读片段池 ⇒ 池里多一条 MIDI 材料，计划一位都不许变"
    );

    let pool_counts: Vec<usize> = two_material
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes().map(|notes| notes.len()))
        .collect();
    let counts = part_note_counts(&absent);
    assert!(
        !counts.is_empty(),
        "project 路径必须真的产出和声材料，一个 `AddClip` 都没有: {counts:?}"
    );
    for count in &counts {
        assert!(
            pool_counts.contains(count),
            "project 路径某声部音符数 {count} 不在池内各条 MIDI 材料的音符数 {pool_counts:?} 里 \
             ⇒ 材料不是从池装配的（`transposed_notes` 与源片段 1:1）"
        );
    }
}
