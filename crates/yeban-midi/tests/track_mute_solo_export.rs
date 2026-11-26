//! 轨级 mute/solo 导出判据（⭐ T-FIX-506，issue ISS-MWWUVNBD-64ED85）。
//!
//! 只经公开 API（`export_from_project` → `to_smf_bytes` → `parse_smf`），
//! 形同 `yeban-mcp` / `yeban-app` 的视角。
//!
//! ## 裁决（与摆放级 `muted` 同款，见 `export.rs` 的映射表）
//!
//! 可闻门 = `!mute && (!any_solo || solo || solo_safe)`，其中 `any_solo` =
//! 任一轨道 `solo == true`（全工程扫，含主总线）；不可闻的轨道整条不导出
//! （不产生 `MTrk`，不占通道号）。口径与引擎 `track_is_audible`
//! （yeban-engine 的 `synth.rs`，`track_gain_scale` 的单一来源）逐字相同。
//!
//! ## 红对照
//!
//! 除 `solo_safe_track_survives_another_tracks_solo`（独奏安全臂，
//! 在不带修法的树上也是绿 —— 它钉的是\"别把 solo_safe 一并静音\"，
//! 防止把修法写成\"只导 solo 轨\"的偷懒版）外，本文件其余判据在不带修法的
//! 树上**红**、补丁树上**绿**（基线核对见回执）。

use yeban_midi::export::{MidiExportError, export_from_project};
use yeban_midi::midi::parse_smf;
use yeban_model::project::{ClipPlacement, LoopConfig, YebanProjectV1};
use yeban_model::{EntityId, TrackKind};

/// 两条可闻 MIDI 轨的工程（演示夹具归一化 + 第二条 MIDI 轨复用同一 MIDI 片段）。
///
/// 演示夹具本身带轨级开关（T1 `solo == true`、T6 `mute == true`），
/// 故先把全部轨道的 `mute` / `solo` / `solo_safe` 清零，再把 T1 的 MIDI
/// 摆放克隆到第二条 MIDI 轨（"铺底"，同片段、同起点）—— 两条轨导出后
/// 各 6 颗音符，唯一区别是通道号（第 0 条 → 通道 0，第 1 条 → 通道 1）。
/// 返回 `(工程, 第一条轨 id, 第二条轨 id)`。
fn two_midi_track_project() -> (YebanProjectV1, EntityId, EntityId) {
    let mut project = yeban_model::samples::demo_project();
    for track in project.tracks.values_mut() {
        track.mute = false;
        track.solo = false;
        track.solo_safe = false;
    }
    let mut midi_tracks: Vec<EntityId> = project
        .tracks
        .values()
        .filter(|track| track.id != project.master_bus_track_id && track.kind == TrackKind::Midi)
        .map(|track| track.id)
        .collect();
    midi_tracks.sort_unstable();
    let (first_id, second_id) = (midi_tracks[0], midi_tracks[1]);
    assert_eq!(project.tracks[&first_id].name, "鼓");
    assert_eq!(project.tracks[&second_id].name, "铺底");
    let placement = *project.tracks[&first_id]
        .clips
        .values()
        .next()
        .expect("演示夹具的第一条 MIDI 轨必有摆放");
    let second_placement_id = EntityId::new();
    project
        .tracks
        .get_mut(&second_id)
        .expect("第二条轨在")
        .clips
        .insert(
            second_placement_id,
            ClipPlacement {
                id: second_placement_id,
                clip_id: placement.clip_id,
                start_tick: placement.start_tick,
                duration_ticks: placement.duration_ticks,
                loop_config: LoopConfig::default(),
                muted: false,
                offset_samples: None,
                source_start_samples: None,
                fade_in_ticks: None,
                fade_out_ticks: None,
            },
        );
    (project, first_id, second_id)
}

/// 某通道的回读音符数。
fn notes_on_channel(bytes: &[u8], channel: u8) -> usize {
    parse_smf(bytes)
        .expect("导出的字节必须能被 SMF 读取面读回")
        .notes
        .iter()
        .filter(|note| note.channel == channel)
        .count()
}

/// 正向控制臂：两条轨都可闻 ⇒ 各 6 颗音符，通道 0 / 1。
#[test]
fn two_audible_tracks_both_export() {
    let (project, _, _) = two_midi_track_project();
    let export = export_from_project(&project).expect("两条可闻轨必须可导出");
    assert_eq!(export.tracks.len(), 2, "两条轨都要导出");
    assert_eq!(export.tracks[0].name, "鼓");
    assert_eq!(export.tracks[1].name, "铺底");
    assert_eq!(export.tracks[0].channel, 0);
    assert_eq!(export.tracks[1].channel, 1);
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(
        notes_on_channel(&bytes, 0),
        6,
        "通道 0 = 第一条轨的 6 颗音符"
    );
    assert_eq!(
        notes_on_channel(&bytes, 1),
        6,
        "通道 1 = 第二条轨的 6 颗音符"
    );
}

/// 判据：静音轨不导出（只剩另一条轨的 6 颗音符）。
#[test]
fn muted_track_exports_no_notes() {
    let (mut project, _, second_id) = two_midi_track_project();
    project.tracks.get_mut(&second_id).expect("轨道在").mute = true;
    let export = export_from_project(&project).expect("另一条轨可闻，导出必须成功");
    assert_eq!(export.tracks.len(), 1, "静音轨不产生 MTrk");
    assert_eq!(export.tracks[0].name, "鼓");
    assert_eq!(export.tracks[0].channel, 0, "只剩一条轨，通道号从 0 起");
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(notes_on_channel(&bytes, 0), 6, "通道 0 只有第一条轨的音符");
    assert_eq!(
        notes_on_channel(&bytes, 1),
        0,
        "静音轨的音符不得出现在任何通道"
    );
}

/// 判据：任一轨道独奏时，只导独奏轨（被独奏压住的轨不导出；通道号重排）。
#[test]
fn solo_track_exports_only_the_solo_track() {
    let (mut project, _, second_id) = two_midi_track_project();
    project.tracks.get_mut(&second_id).expect("轨道在").solo = true;
    let export = export_from_project(&project).expect("独奏轨可闻，导出必须成功");
    assert_eq!(export.tracks.len(), 1, "非独奏轨不产生 MTrk");
    assert_eq!(export.tracks[0].name, "铺底");
    assert_eq!(
        export.tracks[0].channel, 0,
        "独奏轨是唯一导出的轨，通道号为 0"
    );
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(notes_on_channel(&bytes, 0), 6, "通道 0 只有独奏轨的音符");
    assert_eq!(notes_on_channel(&bytes, 1), 0, "被独奏压住的轨不得发声");
}

/// 判据：`mute` 优先于 `solo`（同轨又静音又独奏 ⇒ 不导出）。
#[test]
fn mute_beats_solo_on_the_same_track() {
    let (mut project, first_id, second_id) = two_midi_track_project();
    project.tracks.get_mut(&first_id).expect("轨道在").mute = true;
    project.tracks.get_mut(&first_id).expect("轨道在").solo = true;
    project.tracks.get_mut(&second_id).expect("轨道在").solo = true;
    let export = export_from_project(&project).expect("第二条独奏轨可闻，导出必须成功");
    assert_eq!(export.tracks.len(), 1, "又静音又独奏的轨不产生 MTrk");
    assert_eq!(export.tracks[0].name, "铺底");
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(notes_on_channel(&bytes, 0), 6, "通道 0 只有独奏轨的音符");
    assert_eq!(notes_on_channel(&bytes, 1), 0, "静音轨的音符不得复活");
}

/// 独奏安全臂（**非**红对照：在不带修法的树上也是绿）：`solo_safe` 的轨
/// 不被其它轨的 `solo` 静音 —— 防止把修法写成\"只导 solo 轨\"的偷懒版。
#[test]
fn solo_safe_track_survives_another_tracks_solo() {
    let (mut project, first_id, second_id) = two_midi_track_project();
    project.tracks.get_mut(&first_id).expect("轨道在").solo_safe = true;
    project.tracks.get_mut(&second_id).expect("轨道在").solo = true;
    let export = export_from_project(&project).expect("两条轨都可闻，导出必须成功");
    assert_eq!(export.tracks.len(), 2, "独奏轨 + 独奏安全轨都要导出");
    assert_eq!(export.tracks[0].name, "鼓");
    assert_eq!(export.tracks[1].name, "铺底");
    let bytes = export.to_smf_bytes().expect("编码");
    assert_eq!(notes_on_channel(&bytes, 0), 6, "通道 0 = 独奏安全轨的音符");
    assert_eq!(notes_on_channel(&bytes, 1), 6, "通道 1 = 独奏轨的音符");
}

/// 判据：全部 MIDI 轨都不可闻 ⇒ 精确的 `NoMidiContent`（不写空文件）。
#[test]
fn all_tracks_inaudible_is_no_midi_content() {
    let (mut project, first_id, second_id) = two_midi_track_project();
    project.tracks.get_mut(&first_id).expect("轨道在").mute = true;
    project.tracks.get_mut(&second_id).expect("轨道在").mute = true;
    assert!(
        matches!(
            export_from_project(&project),
            Err(MidiExportError::NoMidiContent)
        ),
        "全部轨静音 ⇒ 无内容"
    );
}
