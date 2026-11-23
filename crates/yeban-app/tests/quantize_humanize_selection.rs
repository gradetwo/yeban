//! 量化 / 人性化尊重音符选区（T-FIX-206；issue ISS-MWTFLV2G-D14798）。
//!
//! 缺陷：`host::humanize_clip_ops_for` / `host::quantize_clip_ops_for` 只认
//! "选中片段"，遍历片段的全部音符 —— 框选 3 颗做量化会把整片段一起量化。
//!
//! 口径（简报定）：
//! 1. 两函数新增 `selected_note_ids: Option<&BTreeSet<EntityId>>` 参数；
//!    `Some` 非空时只处理与片段音符的**交集**，`None` / 空集时保持整片段口径
//!    （向后兼容：MCP 与既有判据走的仍是整片段展开）；
//! 2. 调用点传入 `ui.get_selected_ulids()` 解析出的身份集；
//! 3. 人性化保留片段内的**原序号**作抖动下标 —— 选区内的音符拿到与整片段
//!    人性化时逐位相同的抖动（判据③的等价臂钉住）。
//!
//! 本文件是纯函数判据：直接喂合成工程（5 颗音符的片段），不依赖卷帘多选入口
//! （T-FIX-204 未合入前，选区由程序构造）。

use std::collections::{BTreeMap, BTreeSet};
use yeban_app::host::{humanize_clip_ops_for, quantize_clip_ops_for};
use yeban_model::music::MidiNote;
use yeban_model::ops::Op;
use yeban_model::samples::filled_project;
use yeban_model::{EntityId, YebanProjectV1};

/// 权威工程里**第一个**带音符的 MIDI 片段：（摆放身份 ULID，音轨身份，片段身份）。
///
/// 按 `tracks` 的 `BTreeMap` 键序（确定）⇒ 每次都是同一条
/// （`piano_roll_humanize_quantize_ui.rs` 的 `first_midi_clip` 同款）。
fn first_midi_clip(project: &YebanProjectV1) -> (String, EntityId, EntityId) {
    for (track_id, track) in project.tracks.iter() {
        for placement in track.clips.values() {
            if let Some(entry) = project.clip_pool.get(&placement.clip_id)
                && entry.content.notes().is_some_and(|notes| !notes.is_empty())
            {
                return (
                    placement.id.to_canonical_string(),
                    *track_id,
                    placement.clip_id,
                );
            }
        }
    }
    panic!("filled_project 必须含至少一个带音符的 MIDI 片段");
}

/// 5 颗音符的合成片段：起点全部离格（网格 240、强度 1000 时 5 颗全动）。
/// 返回（工程，摆放身份 ULID，5 颗音符的身份 —— 按片段 `BTreeMap` 键序）。
fn five_note_project() -> (YebanProjectV1, String, Vec<EntityId>) {
    let mut project = filled_project();
    let (placement_ulid, _track_id, clip_id) = first_midi_clip(&project);
    let mut notes = BTreeMap::new();
    for (i, start) in [100_u64, 340, 580, 820, 1060].into_iter().enumerate() {
        let id = EntityId::new();
        notes.insert(id, MidiNote::new(id, start, 60 + i as u8, 120));
    }
    let ids: Vec<EntityId> = notes.keys().copied().collect();
    let entry = project.clip_pool.get_mut(&clip_id).expect("片段必须在池里");
    entry.content = yeban_model::project::ClipContent::Midi { notes };
    (project, placement_ulid, ids)
}

/// 一批 op 里的音符身份（`SetNoteTiming` / `SetNoteExpression` /
/// `ModifyNoteVelocity` 三种都带 `note_id`）。
fn op_note_ids(ops: &[Op]) -> BTreeSet<EntityId> {
    ops.iter()
        .map(|op| match op {
            Op::SetNoteTiming { note_id, .. } => *note_id,
            Op::SetNoteExpression { note_id, .. } => *note_id,
            Op::ModifyNoteVelocity { note_id, .. } => *note_id,
            _ => panic!("本票的展开只应产出音符级 op：{op:?}"),
        })
        .collect()
}

/// 判据 ①：选 2/5 颗量化时只有 2 条 `SetNoteTiming`，且正是选中的 2 颗。
#[test]
fn quantize_selection_of_two_out_of_five_produces_exactly_two_ops() {
    let (project, placement_ulid, ids) = five_note_project();
    let selected: BTreeSet<EntityId> = [ids[0], ids[2]].into_iter().collect();
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, Some(&selected))
        .expect("合成片段必须能量化");
    assert_eq!(
        ops.len(),
        2,
        "选 2/5 颗时只能产 2 条 SetNoteTiming，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert!(
        ops.iter().all(|op| matches!(op, Op::SetNoteTiming { .. })),
        "量化展开只应产 SetNoteTiming：{ops:?}"
    );
    assert_eq!(
        op_note_ids(&ops),
        selected,
        "产出的 op 必须正好对应选中的 2 颗音符"
    );
}

/// 判据 ②：空选区（`None` 与空集）时保持旧口径 —— 5 颗全动。
#[test]
fn quantize_empty_selection_falls_back_to_whole_clip() {
    let (project, placement_ulid, ids) = five_note_project();
    for selection in [None, Some(&BTreeSet::new())] {
        let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, selection)
            .expect("合成片段必须能量化");
        assert_eq!(
            ops.len(),
            5,
            "空选区时必须保持整片段口径（5 条），实得 {} 条：{ops:?}",
            ops.len()
        );
        assert_eq!(
            op_note_ids(&ops),
            ids.iter().copied().collect::<BTreeSet<_>>(),
            "空选区时 5 颗音符必须全被量化"
        );
    }
}

/// 判据 ③：选 2/5 颗人性化时只有 4 条 op（每颗 2 条），且与整片段人性化时
/// 同一颗音符的 op 逐位相同（片段内原序号作抖动下标）；空选区时 10 条。
#[test]
fn humanize_selection_of_two_touches_only_selected_notes() {
    let (project, placement_ulid, ids) = five_note_project();
    let selected: BTreeSet<EntityId> = [ids[1], ids[3]].into_iter().collect();
    let scoped = humanize_clip_ops_for(&project, &placement_ulid, 7, 10, 10, Some(&selected))
        .expect("合成片段必须能人性化");
    assert_eq!(
        scoped.len(),
        4,
        "选 2/5 颗时只能产 4 条 op（每颗 2 条），实得 {} 条：{scoped:?}",
        scoped.len()
    );
    assert_eq!(
        op_note_ids(&scoped),
        selected,
        "产出的 op 必须正好对应选中的 2 颗音符"
    );
    // 与整片段展开逐位对照：选区内的音符拿到的抖动必须与整片段时相同。
    let whole = humanize_clip_ops_for(&project, &placement_ulid, 7, 10, 10, None)
        .expect("合成片段必须能人性化");
    assert_eq!(whole.len(), 10, "空选区时 5 颗 × 2 条 = 10 条");
    for op in &scoped {
        assert!(
            whole.contains(op),
            "选区 op 必须与整片段展开里同一颗音符的 op 逐位相同：{op:?}"
        );
    }
}

/// 判据 ④：选区里混进不属于本片段的身份时只处理交集（陈旧选区不炸、不越界）。
#[test]
fn quantize_ignores_selection_ids_outside_the_clip() {
    let (project, placement_ulid, ids) = five_note_project();
    let foreign = EntityId::new();
    let selected: BTreeSet<EntityId> = [ids[0], ids[4], foreign].into_iter().collect();
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, Some(&selected))
        .expect("合成片段必须能量化");
    assert_eq!(
        ops.len(),
        2,
        "选区含 1 颗外来身份时仍只能产 2 条 op，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert_eq!(
        op_note_ids(&ops),
        [ids[0], ids[4]].into_iter().collect::<BTreeSet<_>>(),
        "外来身份必须被交集滤掉"
    );
}
