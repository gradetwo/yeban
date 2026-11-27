//! **T-FIX-580 判据**：钢琴卷帘音符剪贴板。
//!
//! 复制 2 颗音符 → 粘贴后：工程音符数 +2、片段数不变、粘贴组相对节奏不变、
//! 每个新音符身份全新且其余表现力字段逐位沿用；剪切是"复制 + 一次删除批"，
//! 一步撤销全部回来；目标/落点不合法时返回空批（宿主据此响亮回执，⛔ 不静默改粘片段）。

use std::collections::{BTreeMap, BTreeSet};

use yeban_app::host::{note_clipboard_notes_for, note_cut_payload_for, note_paste_ops_for};
use yeban_model::YebanProjectV1;
use yeban_model::ops::Op;
use yeban_model::{ClipContent, ClipPoolEntry, EntityId};

fn notes_total(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(BTreeMap::len)
        .sum()
}

/// 取一个至少含两颗音符的 MIDI 片段：返回（摆放身份，音符身份，片段池条目数）。
fn midi_fixture(project: &YebanProjectV1) -> (String, Vec<String>, usize) {
    for track in project.tracks.values() {
        for placement in track.clips.values() {
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let Some(notes) = entry.content.notes() else {
                continue;
            };
            if notes.len() < 2 {
                continue;
            }
            return (
                placement.id.to_canonical_string(),
                notes.keys().take(2).map(ToString::to_string).collect(),
                project.clip_pool.len(),
            );
        }
    }
    panic!("夹具必须有一个含 ≥2 音符的 MIDI 片段");
}

fn placement<'p>(
    project: &'p YebanProjectV1,
    placement_ulid: &str,
) -> &'p yeban_model::ClipPlacement {
    project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .find(|placement| placement.id.to_canonical_string() == placement_ulid)
        .expect("摆放必须存在")
}

#[test]
fn copy_two_notes_then_paste_adds_two_notes_and_keeps_clip_count() {
    let project = yeban_model::samples::filled_project();
    let (placement_ulid, selected, clip_entries) = midi_fixture(&project);
    let before_count = notes_total(&project);

    let clipboard = note_clipboard_notes_for(&project, &selected);
    assert_eq!(clipboard.len(), 2, "只复制选中的 2 颗音符");
    assert_eq!(clipboard[0].pitch, clipboard[0].pitch, "副本来自真音符");

    let target = placement(&project, &placement_ulid);
    let playhead = target.start_tick.saturating_add(240);
    let (ops, ids) = note_paste_ops_for(&project, &clipboard, &placement_ulid, playhead);
    assert_eq!(ops.len(), 2, "每颗音符一条 AddNote");
    assert_eq!(ids.len(), 2, "每颗副本一个身份");
    assert!(
        ids.iter()
            .all(|id| !selected.iter().any(|old| old == &id.to_string())),
        "副本必须是全新身份"
    );

    let anchor = clipboard
        .iter()
        .map(|note| note.start_tick)
        .min()
        .expect("非空剪贴板");
    let local_playhead = playhead.saturating_sub(target.start_tick);
    for (index, op) in ops.iter().enumerate() {
        let Op::AddNote { note, .. } = op else {
            panic!("只允许 AddNote");
        };
        let source = &clipboard[index];
        assert_eq!(note.pitch, source.pitch);
        assert_eq!(note.velocity, source.velocity);
        assert_eq!(note.channel, source.channel);
        assert_eq!(
            note.start_tick,
            local_playhead + source.start_tick.saturating_sub(anchor),
            "最早音符对齐播放头，其余相对节奏不变"
        );
    }

    let batch = Op::Batch {
        ops,
        description: "paste notes".to_owned(),
    };
    let mut doc = project.clone();
    batch.apply(&mut doc).expect("整批可应用");
    assert_eq!(notes_total(&doc), before_count + 2, "工程音符数 +2");
    assert_eq!(doc.clip_pool.len(), clip_entries, "粘贴音符不得新建片段");
    let placements_before: usize = project.tracks.values().map(|track| track.clips.len()).sum();
    let placements_after: usize = doc.tracks.values().map(|track| track.clips.len()).sum();
    assert_eq!(placements_after, placements_before, "摆放数也不变");

    batch.apply_inverse(&mut doc).expect("一步撤销整批回来");
    assert_eq!(notes_total(&doc), before_count, "撤销后音符数复原");
}

#[test]
fn cut_copies_full_notes_and_deletes_them_in_one_undoable_batch() {
    let project = yeban_model::samples::filled_project();
    let (_, selected, _) = midi_fixture(&project);
    let before_count = notes_total(&project);

    let (clipboard, ops) = note_cut_payload_for(&project, &selected);
    assert_eq!(clipboard.len(), 2, "剪切先复制 2 颗完整音符");
    assert_eq!(ops.len(), 2, "剪切恰好两条 DeleteNote");
    assert!(
        ops.iter().all(|op| matches!(op, Op::DeleteNote { .. })),
        "剪切只允许 DeleteNote"
    );

    let batch = Op::Batch {
        ops,
        description: "cut notes".to_owned(),
    };
    let mut doc = project.clone();
    batch.apply(&mut doc).expect("整批可应用");
    assert_eq!(notes_total(&doc), before_count - 2, "工程音符数 -2");
    assert_eq!(
        doc.clip_pool.len(),
        project.clip_pool.len(),
        "剪切不得删片段"
    );

    batch.apply_inverse(&mut doc).expect("一步撤销全部回来");
    assert_eq!(notes_total(&doc), before_count, "撤销后音符数复原");
}

#[test]
fn paste_refuses_playhead_outside_target_and_empty_clipboard() {
    let project = yeban_model::samples::filled_project();
    let (placement_ulid, selected, _) = midi_fixture(&project);
    let clipboard = note_clipboard_notes_for(&project, &selected);
    let target = placement(&project, &placement_ulid);

    let outside = target
        .start_tick
        .saturating_add(target.duration_ticks)
        .saturating_add(1);
    let (ops, ids) = note_paste_ops_for(&project, &clipboard, &placement_ulid, outside);
    assert!(ops.is_empty(), "播放头不在片段内 ⇒ 空批");
    assert!(ids.is_empty());

    let (ops, ids) = note_paste_ops_for(&project, &[], &placement_ulid, target.start_tick);
    assert!(ops.is_empty(), "空剪贴板 ⇒ 空批");
    assert!(ids.is_empty());

    let (ops, ids) = note_paste_ops_for(&project, &clipboard, "not-a-ulid", target.start_tick);
    assert!(ops.is_empty(), "无效目标片段 ⇒ 空批");
    assert!(ids.is_empty());
}

/// ⭐ T-FIX-971：选区是身份集合，工程里同一个 note id 被两个片段复用时，
/// 复制集必须仍等于选区（一个 id 一份副本），不能按片段全表串计成 10 颗。
#[test]
fn copy_count_equals_selected_ids_even_if_notes_are_duplicated_across_clips() {
    let mut project = yeban_model::samples::filled_project();
    let (_, selected, _) = midi_fixture(&project);
    assert_eq!(selected.len(), 2, "夹具选中两颗音符");

    // 在另一个片段池条目里放入同一批 note id（真人模拟的重复 id 工程）。
    let duplicated: Vec<_> = project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().cloned())
        .take(1)
        .collect();
    assert_eq!(duplicated.len(), 1, "必须能取到一颗用于制造重复 id 的音符");
    let extra_id = EntityId::new();
    project.clip_pool.insert(
        extra_id,
        ClipPoolEntry {
            id: extra_id,
            name: "duplicate-id-clip".to_owned(),
            content: ClipContent::Midi {
                notes: duplicated.into_iter().map(|note| (note.id, note)).collect(),
            },
        },
    );

    let clipboard = note_clipboard_notes_for(&project, &selected);
    assert_eq!(
        clipboard.len(),
        selected.len(),
        "复制数必须等于选区数（重复 id 不得串计）: {clipboard:?}"
    );
    let ids: BTreeSet<_> = clipboard.iter().map(|note| note.id).collect();
    assert_eq!(ids.len(), selected.len(), "每个选中 id 只产出一份副本");

    // 红对照：把 host.rs 的 `seen.insert(*note_id)` 去重去掉 ⇒ 本条红在 clipboard.len() == 3。
}
