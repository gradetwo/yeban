//! 缺口补全（审计 `N9`）：`yeban_propose_section` 此前**每段必开新轨** ——
//! `part_plans` 给每个声部确定性派生一条新音轨，同一首歌连写四段就会得到
//! 四组"Bass/Tenor/Soprano"重复轨。本票加 `partTracks`：ULID 数组，第 `i`
//! 项是第 `i` 个声部要复用的既有音轨。
//!
//! 复用的轨**不**追加 `Op::AddTrack`、不改名不改电平；路由按现状补齐
//! （节点缺席补 `AddRoutingNode`、缺 `TrackToBus` 边补 `ConnectRouting`，
//! 已在位的跳过）。
//!
//! 三类失败分开报（⛔ 折叠成一条"音轨不可用"）：长度不符 / 重复项 ⇒
//! `INVALID_PARAMETER_RANGE`；不存在 / 存在但不是 MIDI 轨 ⇒ `TRACK_NOT_FOUND`。

use std::str::FromStr as _;

use yeban_mcp::domain::section_build::{
    BuildCode, BuildFault, PlanOptions, SectionPlan, plan_with_options,
};
use yeban_model::ids::EntityId;
use yeban_model::{Op, TrackKind, samples::filled_project};

const STYLE: &str = "synthwave"; // 3 声部

/// 由夹具扩展出的工程：原样保留全部内容，再克隆 lead 轨成第三条 MIDI 轨
/// （`routing_graph` 里没有它的节点/边 ⇒ 可以测出"缺席补齐"那一支）。
fn three_track_project() -> (yeban_model::YebanProjectV1, [EntityId; 3]) {
    let mut project = filled_project();
    let lead_id = project
        .tracks
        .iter()
        .find(|(_, track)| track.kind == TrackKind::Midi)
        .map(|(id, _)| *id)
        .expect("夹具必须有一条 MIDI 轨");
    // 第二条复用轨：克隆 lead，**并把它接进路由图**（节点 + TrackToBus → 主总线），
    // 覆盖"已在位就跳过"那一支；第三条只加轨不进路由图，覆盖"缺席补齐"那一支。
    let bass_id = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAV").unwrap();
    let third_id = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAW").unwrap();
    let bus_id = project
        .tracks
        .iter()
        .find(|(_, track)| track.kind == TrackKind::Master)
        .map(|(id, _)| *id)
        .expect("夹具必须有主总线");
    // ⛔ 克隆 lead 会把嵌着旧 track_id 的自动化/宏一起拷过来（校验报
    // `AutomationLaneTargetMismatch`）⇒ 复用轨一律按"干净的新轨"构造。
    project.tracks.insert(
        bass_id,
        yeban_model::TrackV3 {
            id: bass_id,
            name: "reused-bass".to_owned(),
            kind: TrackKind::Midi,
            ..yeban_model::TrackV3::default()
        },
    );
    project.tracks.insert(
        third_id,
        yeban_model::TrackV3 {
            id: third_id,
            name: "reused-third".to_owned(),
            kind: TrackKind::Midi,
            ..yeban_model::TrackV3::default()
        },
    );
    project.routing_graph.nodes.push(bass_id);
    project.routing_graph.edges.insert(
        EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAX").unwrap(),
        yeban_model::RoutingEdge {
            id: EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAX").unwrap(),
            source_node: bass_id,
            destination_node: bus_id,
            kind: yeban_model::RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("扩展夹具必须合法");
    (project, [lead_id, bass_id, third_id])
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
            material_clip_ids: None,
            part_tracks: ids.as_deref(),
        },
    )
}

/// 判据 1：复用轨**不**新建 —— ops 里没有 `AddTrack`；`AddClipPlacement`
/// 逐一落在指名的三条轨上（保序）；已连总线的两条轨不再加边、缺席的第
/// 三条轨补齐 `AddRoutingNode` + `ConnectRouting`。
#[test]
fn reused_tracks_get_placements_without_new_tracks() {
    let (project, [lead_id, bass_id, third_id]) = three_track_project();
    let section_plan =
        try_plan(&project, Some(vec![lead_id, bass_id, third_id])).expect("复用既有轨必须能规划");

    assert_eq!(
        section_plan.part_track_ids,
        vec![lead_id, bass_id, third_id],
        "part_track_ids 必须逐字回显指名表"
    );
    assert!(
        !section_plan
            .ops
            .iter()
            .any(|op| matches!(op, Op::AddTrack { .. })),
        "复用轨不得再追加 AddTrack"
    );
    let placement_targets: Vec<EntityId> = section_plan
        .ops
        .iter()
        .filter_map(|op| {
            let Op::AddClipPlacement { track_id, .. } = op else {
                return None;
            };
            Some(*track_id)
        })
        .collect();
    assert_eq!(
        placement_targets,
        vec![lead_id, bass_id, third_id],
        "摆放必须按声部序落在指名轨上"
    );
    let new_edges: Vec<(EntityId, EntityId)> = section_plan
        .ops
        .iter()
        .filter_map(|op| {
            let Op::ConnectRouting { edge } = op else {
                return None;
            };
            Some((edge.source_node, edge.destination_node))
        })
        .collect();
    assert_eq!(
        new_edges.len(),
        1,
        "lead/bass 已连主总线 ⇒ 只有第三条缺席轨要补边"
    );
    assert_eq!(new_edges[0].0, third_id);
    let new_nodes: Vec<EntityId> = section_plan
        .ops
        .iter()
        .filter_map(|op| {
            let Op::AddRoutingNode { node } = op else {
                return None;
            };
            Some(*node)
        })
        .collect();
    assert!(
        new_nodes.contains(&third_id),
        "缺席路由图的复用轨要补节点: {new_nodes:?}"
    );
    assert!(
        !new_nodes.contains(&lead_id) && !new_nodes.contains(&bass_id),
        "已在路由图里的复用轨不得重复加节点: {new_nodes:?}"
    );
}

/// 判据 2：三类失败分开报 —— 长度不符带 `received`/`requiredParts`；
/// 不存在带 `trackId`/`trackIds`；非 MIDI 轨带 `trackKind`/`acceptedKind`。
#[test]
fn part_tracks_failures_are_reported_per_class() {
    let (project, [lead_id, bass_id, third_id]) = three_track_project();
    // 长度不符（只给 2 条而声部是 3）。
    let fault = try_plan(&project, Some(vec![lead_id, bass_id])).expect_err("长度不符必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["received"], 2);
    assert_eq!(data["requiredParts"], 3);

    // 不存在：一个合法但不在 tracks 里的 ULID。
    let ghost = EntityId::from_str("01ARZ3NDEKTSV4RRFFQ69G5FAZ").unwrap();
    let fault =
        try_plan(&project, Some(vec![lead_id, ghost, third_id])).expect_err("不存在的音轨必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::TrackNotFound));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["trackId"], ghost.to_canonical_string());

    // 存在但不是 MIDI：夹具里的 aux return 轨。
    let aux_id = project
        .tracks
        .iter()
        .find(|(_, track)| track.kind != TrackKind::Midi)
        .map(|(id, _)| *id)
        .expect("夹具必须有一条非 MIDI 轨");
    let fault =
        try_plan(&project, Some(vec![lead_id, aux_id, third_id])).expect_err("非 MIDI 轨必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::TrackNotFound));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["trackId"], aux_id.to_canonical_string());
    assert_eq!(data["acceptedKind"], "Midi");

    // 重复项：同一轨喂两个声部 ⇒ 边身份必然冲突 ⇒ 响亮拒。
    let fault =
        try_plan(&project, Some(vec![lead_id, lead_id, third_id])).expect_err("重复音轨必须拒");
    assert_eq!(fault.domain_code(), Some(BuildCode::InvalidParameterRange));
    let BuildFault::Domain { data, .. } = &fault else {
        panic!()
    };
    assert_eq!(data["trackId"], lead_id.to_canonical_string());
}

/// 判据 3：复用轨与 theory 路径**正交** —— `materialSource:"theory"` 配
/// `partTracks` 仍然成立（复用轨管目标、材料源管内容）。
#[test]
fn part_tracks_work_with_theory_source_too() {
    let (project, [lead_id, bass_id, third_id]) = three_track_project();
    let ids = vec![lead_id, bass_id, third_id];
    let section_plan = plan_with_options(
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
            material_clip_ids: None,
            part_tracks: Some(ids.as_slice()),
        },
    )
    .expect("theory 路径配复用轨必须能规划");
    assert!(
        !section_plan
            .ops
            .iter()
            .any(|op| matches!(op, Op::AddTrack { .. })),
        "theory 路径下复用轨同样不得再建"
    );
}
