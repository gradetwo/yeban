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
use yeban_app::host::{clear_micro_timing_ops_for, humanize_clip_ops_for, quantize_clip_ops_for};
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

/// ⭐ T-FIX-767 夹具：把两个 MIDI 片段的首颗音符钉到离格起点（100 tick）。
/// 返回（工程，片段①的音符身份，片段②的音符身份）。
fn two_offgrid_clips() -> (YebanProjectV1, Vec<EntityId>, Vec<EntityId>) {
    let mut project = filled_project();
    // (track_id, placement_id, clip_id) —— 只收带音符的 MIDI 片段。
    let mut clips: Vec<(EntityId, EntityId, EntityId)> = Vec::new();
    for (track_id, track) in project.tracks.iter() {
        for placement in track.clips.values() {
            if project
                .clip_pool
                .get(&placement.clip_id)
                .and_then(|entry| entry.content.notes())
                .is_some_and(|notes| !notes.is_empty())
            {
                clips.push((*track_id, placement.id, placement.clip_id));
            }
        }
    }
    // filled_project 只带一个 MIDI 片段 ⇒ 复制成第二个，供跨片段选区判据使用。
    if clips.len() == 1 {
        let (track_id, placement_id, clip_id) = clips[0];
        let new_clip_id = EntityId::new();
        let new_placement_id = EntityId::new();
        let entry = project.clip_pool.get(&clip_id).expect("MIDI 片段").clone();
        project.clip_pool.insert(
            new_clip_id,
            yeban_model::project::ClipPoolEntry {
                id: new_clip_id,
                name: "Clip B".to_owned(),
                ..entry
            },
        );
        let placement = *project
            .tracks
            .get(&track_id)
            .expect("音轨")
            .clips
            .get(&placement_id)
            .expect("摆放");
        project
            .tracks
            .get_mut(&track_id)
            .expect("音轨")
            .clips
            .insert(
                new_placement_id,
                yeban_model::project::ClipPlacement {
                    id: new_placement_id,
                    ..placement
                },
            );
        clips.push((track_id, new_placement_id, new_clip_id));
    }
    assert!(clips.len() >= 2, "必须构造出两个 MIDI 片段");
    let mut groups = Vec::new();
    for (_, _, clip_id) in clips.iter().take(2) {
        let entry = project.clip_pool.get_mut(clip_id).expect("MIDI 片段");
        let notes = entry.content.notes_mut().expect("MIDI 音符");
        let ids: Vec<EntityId> = notes.keys().copied().collect();
        if let Some(first) = ids.first()
            && let Some(note) = notes.get_mut(first)
        {
            note.start_tick = 100;
        }
        groups.push(ids);
    }
    (project, groups[0].clone(), groups[1].clone())
}

/// ⭐ 判据⑤（T-FIX-767）：无选中片段（作用域 = 全部音符，与 `roll_scope() == None`
/// 同源）时量化必须生效 —— 旧实现此处直接 Err("未选中剪辑")。
#[test]
fn quantize_with_no_selected_clip_quantifies_all_notes() {
    let (project, first, second) = two_offgrid_clips();
    let all: BTreeSet<EntityId> = first.iter().chain(second.iter()).copied().collect();
    let ops = quantize_clip_ops_for(&project, "", 240, 1000, Some(&all))
        .expect("空 selected_clip_ulid 必须按全部音符作用域展开");
    let changed = op_note_ids(&ops);
    assert!(
        changed.contains(&first[0]) && changed.contains(&second[0]),
        "两个片段的离格音符都必须被量化，实得 {changed:?}"
    );
}

/// ⭐ 判据⑥（T-FIX-767）：跨片段选区量化两边都变，⛔ 不再静默漏掉另一片段。
#[test]
fn quantize_cross_clip_selection_touches_both_clips() {
    let (project, first, second) = two_offgrid_clips();
    let selected: BTreeSet<EntityId> = [first[0], second[0]].into_iter().collect();
    let ops = quantize_clip_ops_for(&project, "", 240, 1000, Some(&selected))
        .expect("跨片段选区必须按全部音符作用域展开");
    assert_eq!(op_note_ids(&ops), selected, "跨片段选中的两颗都必须被量化");
}

/// ⭐ 判据⑦：人性化与量化同源 —— 无选中片段时跨片段选区两边都产出 op。
#[test]
fn humanize_no_selected_clip_touches_both_clips() {
    let (project, first, second) = two_offgrid_clips();
    let selected: BTreeSet<EntityId> = [first[0], second[0]].into_iter().collect();
    let ops = humanize_clip_ops_for(&project, "", 7, 10, 10, Some(&selected))
        .expect("空 selected_clip_ulid 必须按全部音符作用域展开");
    assert_eq!(
        op_note_ids(&ops),
        selected,
        "人性化跨片段选区两边都必须产出 op"
    );
}

/// T-FIX-675：5 颗带微时值的音符（起点与 `five_note_project` 相同，全部离格）。
/// 微时值：`Some(100)` / `Some(-30)` / `Some(1)`（50% 时缩到 0 ⇒ 归一为 `None`）/
/// `None` / `Some(240)`。返回（工程，摆放身份 ULID，5 颗音符的身份 —— 按片段
/// `BTreeMap` 键序）。
fn micro_note_project() -> (YebanProjectV1, String, Vec<EntityId>) {
    let (mut project, placement_ulid, _ids) = five_note_project();
    let clip_id = first_midi_clip(&project).2;
    let entry = project.clip_pool.get_mut(&clip_id).expect("片段必须在池里");
    let Some(notes) = entry.content.notes_mut() else {
        panic!("片段必须是 MIDI");
    };
    // `BTreeMap` 按 ULID 键序（⛔ 不是插入序）⇒ 先按起点排好序再赋微时值，
    // 返回的 ids[0..5] 对应起点 100/340/580/820/1060。
    let mut by_start: Vec<(u64, EntityId)> = notes
        .iter()
        .map(|(id, note)| (note.start_tick, *id))
        .collect();
    by_start.sort();
    let ids: Vec<EntityId> = by_start.iter().map(|(_, id)| *id).collect();
    for (id, micro) in ids
        .iter()
        .copied()
        .zip([Some(100_i16), Some(-30), Some(1), None, Some(240)])
    {
        notes
            .get_mut(&id)
            .expect("音符必须在片段里")
            .micro_timing_ticks = micro;
    }
    (project, placement_ulid, ids)
}

/// 该音符的微时值 op（`SetNoteExpression` 里新微时值），没有 ⇒ `None`。
fn micro_op_of(ops: &[Op], note_id: EntityId) -> Option<Option<i16>> {
    ops.iter().find_map(|op| match op {
        Op::SetNoteExpression {
            note_id: id,
            new_micro_timing_ticks,
            ..
        } if *id == note_id => Some(*new_micro_timing_ticks),
        _ => None,
    })
}

/// 该音符的起点 op（`SetNoteTiming` 里新起点），没有 ⇒ `None`。
fn timing_op_of(ops: &[Op], note_id: EntityId) -> Option<u64> {
    ops.iter().find_map(|op| match op {
        Op::SetNoteTiming {
            note_id: id,
            new_start_tick,
            ..
        } if *id == note_id => Some(*new_start_tick),
        _ => None,
    })
}

/// 判据 ⑤（T-FIX-675）：强度 100% 时 5 颗全到网格、微时值全清为 `None`。
/// 每颗音符 `SetNoteTiming` 在前、`SetNoteExpression` 在后；`None` 的那颗不产
/// 表达式 op（9 条 = 5 起点 + 4 微时值）。
#[test]
fn quantize_full_strength_clears_micro_timing_to_none() {
    let (project, placement_ulid, ids) = micro_note_project();
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, None)
        .expect("合成片段必须能量化");
    assert_eq!(
        ops.len(),
        9,
        "100% 时 5 起点 + 4 微时值 = 9 条 op，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert_eq!(
        timing_op_of(&ops, ids[0]),
        Some(0),
        "起点 100 在网格 240 下 100% 吸附到 0"
    );
    assert_eq!(timing_op_of(&ops, ids[1]), Some(240));
    assert_eq!(timing_op_of(&ops, ids[2]), Some(480));
    assert_eq!(timing_op_of(&ops, ids[3]), Some(720));
    assert_eq!(timing_op_of(&ops, ids[4]), Some(960));
    for id in [ids[0], ids[1], ids[2], ids[4]] {
        assert_eq!(
            micro_op_of(&ops, id),
            Some(None),
            "100% 时微时值必须清为 None：{id:?}"
        );
    }
    assert_eq!(
        micro_op_of(&ops, ids[3]),
        None,
        "本来就是 None 的音符不产表达式 op"
    );
    // 同一颗音符两条 op 的顺序固定是起点在前、表达式在后（按音符分组断言：
    // 展开按 `BTreeMap` 键序产 op，全局顺序与起点序无关，只钉组内顺序）。
    let mut kinds_by_note: BTreeMap<EntityId, Vec<&str>> = BTreeMap::new();
    for op in &ops {
        let (note_id, kind) = match op {
            Op::SetNoteTiming { note_id, .. } => (*note_id, "timing"),
            Op::SetNoteExpression { note_id, .. } => (*note_id, "expression"),
            _ => panic!("量化展开只应产这两类 op：{op:?}"),
        };
        kinds_by_note.entry(note_id).or_default().push(kind);
    }
    for id in [ids[0], ids[1], ids[2], ids[4]] {
        assert_eq!(
            kinds_by_note[&id],
            ["timing", "expression"],
            "带微时值的音符必须先产 Timing 再产 Expression：{id:?}"
        );
    }
    assert_eq!(
        kinds_by_note[&ids[3]],
        ["timing"],
        "无微时值的音符只产 Timing"
    );
}

/// 判据 ⑥（T-FIX-675）：强度 50% 时微时值约减半（向零取整）；`Some(1)` 缩到
/// 0 ⇒ 归一为 `None`；起点同样插值一半。
#[test]
fn quantize_half_strength_halves_micro_timing() {
    let (project, placement_ulid, ids) = micro_note_project();
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 500, None)
        .expect("合成片段必须能量化");
    assert_eq!(
        ops.len(),
        9,
        "50% 时同样 5 起点 + 4 微时值 = 9 条 op，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert_eq!(micro_op_of(&ops, ids[0]), Some(Some(50)), "100 @50% ⇒ 50");
    assert_eq!(micro_op_of(&ops, ids[1]), Some(Some(-15)), "-30 @50% ⇒ -15");
    assert_eq!(
        micro_op_of(&ops, ids[2]),
        Some(None),
        "1 @50% 缩到 0 ⇒ 归一为 None"
    );
    assert_eq!(micro_op_of(&ops, ids[4]), Some(Some(120)), "240 @50% ⇒ 120");
    assert_eq!(timing_op_of(&ops, ids[0]), Some(50), "起点 100 @50% ⇒ 50");
    assert_eq!(
        timing_op_of(&ops, ids[4]),
        Some(1010),
        "起点 1060 @50% ⇒ 1010"
    );
}

/// 判据 ⑦（T-FIX-675）：强度 0 时展开为空（起点不动、微时值不动）；已在格上
/// 但带微时值的音符只产一条表达式 op（起点不动）。
#[test]
fn quantize_zero_strength_changes_nothing_and_on_grid_note_yields_expression_only() {
    let (project, placement_ulid, _ids) = micro_note_project();
    let ops =
        quantize_clip_ops_for(&project, &placement_ulid, 240, 0, None).expect("合成片段必须能量化");
    assert!(
        ops.is_empty(),
        "强度 0 时展开必须为空，实得 {} 条：{ops:?}",
        ops.len()
    );

    // 已在格上的音符（起点 480）带微时值 ⇒ 100% 时只产表达式 op。
    let (mut project, placement_ulid, ids) = five_note_project();
    let clip_id = first_midi_clip(&project).2;
    let entry = project.clip_pool.get_mut(&clip_id).expect("片段必须在池里");
    let Some(notes) = entry.content.notes_mut() else {
        panic!("片段必须是 MIDI");
    };
    let note = notes.get_mut(&ids[0]).expect("音符必须在片段里");
    note.start_tick = 480;
    note.micro_timing_ticks = Some(80);
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, None)
        .expect("合成片段必须能量化");
    // 其余 4 颗离格音符各 1 条起点 op；第 1 颗只产表达式 op。
    assert_eq!(
        ops.len(),
        5,
        "1 表达式 + 4 起点 = 5 条 op，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert_eq!(
        timing_op_of(&ops, ids[0]),
        None,
        "已在格上的音符不产起点 op"
    );
    assert_eq!(
        micro_op_of(&ops, ids[0]),
        Some(None),
        "已在格上的音符微时值仍清为 None"
    );
}

/// 判据 ⑧（T-FIX-675）：量化尊重选区 —— 只清选区内音符的微时值，选区外不动。
#[test]
fn quantize_micro_timing_respects_selection() {
    let (project, placement_ulid, ids) = micro_note_project();
    let selected: BTreeSet<EntityId> = [ids[0], ids[2]].into_iter().collect();
    let ops = quantize_clip_ops_for(&project, &placement_ulid, 240, 1000, Some(&selected))
        .expect("合成片段必须能量化");
    assert_eq!(
        op_note_ids(&ops),
        selected,
        "选区外音符的微时值必须一位不动：{ops:?}"
    );
    assert_eq!(
        micro_op_of(&ops, ids[0]),
        Some(None),
        "选区内音符的微时值必须清掉"
    );
    assert_eq!(
        micro_op_of(&ops, ids[2]),
        Some(None),
        "选区内音符的微时值必须清掉"
    );
}

/// 判据 ⑨（T-FIX-675）：清除动作只清作用范围内的微时值 —— 整片段 4 条、
/// 选区 2 条、选区内无微时值时空展开；概率 / 连击原样、旧值原样进 op（一步
/// 撤销即按 `old_*` 还回）。
#[test]
fn clear_micro_timing_ops_scope_and_undo_payload() {
    let (mut project, placement_ulid, ids) = micro_note_project();
    // 给第 1 颗音符挂概率 / 连击：清除 op 必须原样带走它们。
    let clip_id = first_midi_clip(&project).2;
    let entry = project.clip_pool.get_mut(&clip_id).expect("片段必须在池里");
    let Some(notes) = entry.content.notes_mut() else {
        panic!("片段必须是 MIDI");
    };
    let note = notes.get_mut(&ids[0]).expect("音符必须在片段里");
    note.probability = Some(0.5);
    note.ratchet = Some(2);

    let ops =
        clear_micro_timing_ops_for(&project, &placement_ulid, None).expect("合成片段必须能清除");
    assert_eq!(
        ops.len(),
        4,
        "整片段 4 颗带微时值的音符 ⇒ 4 条 op，实得 {} 条：{ops:?}",
        ops.len()
    );
    assert!(
        ops.iter().all(|op| matches!(
            op,
            Op::SetNoteExpression {
                new_micro_timing_ticks: None,
                ..
            }
        )),
        "清除 op 必须全是 new_micro_timing_ticks: None 的 SetNoteExpression：{ops:?}"
    );
    let first = ops
        .iter()
        .find(|op| matches!(op, Op::SetNoteExpression { note_id, .. } if *note_id == ids[0]))
        .expect("第 1 颗音符必须有清除 op");
    let Op::SetNoteExpression {
        old_micro_timing_ticks,
        old_probability,
        old_ratchet,
        new_probability,
        new_ratchet,
        ..
    } = first
    else {
        panic!("第 1 颗音符的 op 必须是 SetNoteExpression");
    };
    assert_eq!(
        *old_micro_timing_ticks,
        Some(100),
        "旧微时值进 op，撤销时原样还回"
    );
    assert_eq!(*old_probability, Some(0.5), "概率原样进 op");
    assert_eq!(*old_ratchet, Some(2), "连击原样进 op");
    assert_eq!(*new_probability, Some(0.5), "概率新值不变");
    assert_eq!(*new_ratchet, Some(2), "连击新值不变");
    assert_eq!(
        op_note_ids(&ops),
        [ids[0], ids[1], ids[2], ids[4]]
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "本来就是 None 的第 4 颗不产 op"
    );

    // 选区只清选区内。
    let selected: BTreeSet<EntityId> = [ids[1], ids[4]].into_iter().collect();
    let scoped = clear_micro_timing_ops_for(&project, &placement_ulid, Some(&selected))
        .expect("合成片段必须能清除");
    assert_eq!(
        op_note_ids(&scoped),
        selected,
        "选区外音符的微时值必须一位不动：{scoped:?}"
    );

    // 选区内没有带微时值的音符 ⇒ 空展开（调用方据此给明文、不提交）。
    let empty_selected: BTreeSet<EntityId> = [ids[3]].into_iter().collect();
    let empty = clear_micro_timing_ops_for(&project, &placement_ulid, Some(&empty_selected))
        .expect("合成片段必须能清除");
    assert!(
        empty.is_empty(),
        "选区内无微时值时展开必须为空，实得 {empty:?}"
    );

    // 整片段都没有微时值 ⇒ 空展开。
    let (project, placement_ulid, _ids) = five_note_project();
    let empty =
        clear_micro_timing_ops_for(&project, &placement_ulid, None).expect("合成片段必须能清除");
    assert!(
        empty.is_empty(),
        "整片段无微时值时展开必须为空，实得 {empty:?}"
    );
}
