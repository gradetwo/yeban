//! 谱面特征提取：`YebanProjectV1` → [`yeban_dsp::song_overview::ScoreInput`]。
//!
//! 整曲多层可视化总览图（设计稿 `yeban-songmap-design.md` Phase 1）的谱面侧入口：
//! 把持久化工程投影成 dsp 分析线的输入契约。本模块是**唯一**一份投影实现 ——
//! 渲染侧（`yeban-engine` 的快照投影）与 MCP 各自的音符展开不得另写一份
//! （两份必然漂移，见 ADR-0001 D28）。
//!
//! ## 与引擎调度表的同一口径
//!
//! 音符展开逐字对齐 `yeban-engine` 的 `project_schedules`（快照投影）：
//!
//! - 摆放静音 ⇒ 跳过；片段池里找不到的摆放 ⇒ 跳过；
//! - 触发判定只走 [`MidiNote::triggers`]（种子 = 工程 `rng_seed`）；
//! - 循环展开只走 [`ClipPlacement::note_copies`]（ADR-0010 D1–D3）；
//! - 连击（ratchet）把时值整数等分成 N 个脉冲（余数不补），脉冲起点含微时值偏移、
//!   夹到摆放区间内，尾巴裁到摆放末端；
//! - 力度 0 的音符**保留**（引擎同样把它排进调度表、只以增益 0 静音）。
//!
//! 与引擎**不同**的两处（有意，且各有理由）：
//!
//! - **可闻门**：静音轨、或有独奏时非独奏且非 `solo_safe` 的轨，音符**不进**
//!   `ScoreInput`。引擎把它们排进调度表但乘增益 0 —— 谱面侧若保留，疏密层会
//!   把一段实际静音的段落算成高密度，与渲染音频对不上（还会误触发
//!   `compression_squash` 诊断）。口径与 MCP `track_is_audible` 同一句谓词。
//! - **引擎的单轨调度容量上限**（`MAX_NOTES_PER_TRACK`）**不**在这里复现：
//!   该常量住在 `yeban-engine`，模型层够不着它。超过上限的工程，谱面侧会比
//!   实际渲染多列音符 —— Phase 1 接受这一缺口（演示工程与民谣 B 均远低于上限）。
//!
//! ## 其它映射口径
//!
//! - 时间一律用**拍**：`tick / PPQ`（PPQ = 960）。速度图恒含 0 拍条目（工程
//!   `bpm`），`tempo_map` 的点按 tick 升序换算；非法速度（非有限、≤ 0）丢弃，
//!   丢完 0 拍条目时 dsp 侧回退到 120 BPM（[`ScoreInput`] 的约定）。
//! - 每小节拍数 = `numerator × 4 / denominator`（四分音符拍数）：4/4 → 4，
//!   6/8 → 3。`time_signature_map` 的拍号变化 Phase 1 不展开（dsp 契约只有
//!   一个 `beats_per_bar`），整曲按顶层拍号计。
//! - 调号：升降号数沿五度圈换算主音（`sharps × 7 mod 12`），小调取关系小调
//!   （主音 − 3 半音）；无调号 ⇒ C 大调。
//! - 段落 Marker：`sections` 按起始 tick 升序，小节号 = `floor(起始拍 /
//!   每小节拍数) + 1`；同一小节多个段落只留最早一个。
//! - 打击轨判定（Phase 1 启发式，模型里**没有**打击轨标记）：采样器绑定路径
//!   含 `drum` 或 `perc`（不分大小写），或任一音符的 MIDI 通道为 9（通道 10）。
//!   内置鼓机设备（`DrumsParams`）的轨同样走这条判定 —— 它的设备名不带
//!   `drum` 字样时会被算成旋律轨，缺口登记在模块文档里，不静默猜测。
//! - 输出音符按 `(起始拍, 音高, 时值, 轨道)` 排序：确定性（`BTreeMap` 的身份
//!   序不等于时间序），dsp 侧无序也可，但稳定输出便于对照与测试。

use crate::ids::PPQ;
use crate::music::MidiNote;
use crate::project::{ClipContent, KeySignature, TrackV3, YebanProjectV1};
use yeban_dsp::song_overview::{NoteEvent, ScoreInput, SectionMarker, TempoChange};

/// 把一份工程投影成 dsp 分析的谱面输入。
///
/// 确定性：同一份工程恒得同一份 `ScoreInput`（`BTreeMap` 迭代序 + 末尾排序）。
/// 空工程 ⇒ 空音符、空速度图（dsp 按 120 BPM / 4/4 计）、无 Marker。
#[must_use]
pub fn score_input_from_project(project: &YebanProjectV1) -> ScoreInput {
    let beats_per_bar = beats_per_bar(project);
    let (key_tonic_pc, is_minor) = key_tonic_pc(project.key_signature);
    ScoreInput {
        notes: project_notes(project),
        tempo_map: project_tempo_map(project),
        beats_per_bar,
        key_tonic_pc,
        is_minor,
        markers: project_markers(project, beats_per_bar),
    }
}

/// 调号 → （主音音高类别，是否小调）。
///
/// 五度圈：升号数 `s` 的大调主音 = `s × 7 mod 12`（C=0）；小调取关系小调
/// （主音 − 3 半音，即 `+ 9 mod 12`）。无调号 ⇒ C 大调 `(0, false)`。
#[must_use]
pub fn key_tonic_pc(key: Option<KeySignature>) -> (u8, bool) {
    let Some(key) = key else {
        return (0, false);
    };
    let major_tonic = (i32::from(key.sharps_flats) * 7).rem_euclid(12);
    if key.is_minor {
        (
            u8::try_from((major_tonic + 9).rem_euclid(12)).unwrap_or(0),
            true,
        )
    } else {
        (u8::try_from(major_tonic).unwrap_or(0), false)
    }
}

/// 每小节拍数（四分音符拍数）：`numerator × 4 / denominator`。
#[must_use]
fn beats_per_bar(project: &YebanProjectV1) -> u32 {
    let numerator = u32::from(project.time_signature.numerator);
    let denominator = u32::from(project.time_signature.denominator);
    if denominator == 0 {
        return 4;
    }
    (numerator.saturating_mul(4) / denominator).max(1)
}

/// 可闻门（与 MCP `track_is_audible` 同一句谓词）：静音轨不发声；有独奏时
/// 只有独奏轨与 `solo_safe` 轨发声。
#[must_use]
fn track_is_audible(track: &TrackV3, any_solo: bool) -> bool {
    !track.mute && (!any_solo || track.solo || track.solo_safe)
}

/// 打击轨判定（Phase 1 启发式，见模块文档）：采样器绑定路径含 `drum`/`perc`。
#[must_use]
fn track_is_percussion(track: &TrackV3) -> bool {
    track.sampler.as_ref().is_some_and(|binding| {
        let path = binding.path.to_ascii_lowercase();
        path.contains("drum") || path.contains("perc")
    })
}

/// tick → 拍（`PPQ` = 960 tick/拍）。
#[must_use]
fn ticks_to_beats(tick: u64) -> f32 {
    tick as f32 / PPQ as f32
}

/// 工程的全部音符事件（可闻轨、已触发、循环展开、连击拆分、裁到摆放末端）。
fn project_notes(project: &YebanProjectV1) -> Vec<NoteEvent> {
    let any_solo = project.tracks.values().any(|track| track.solo);
    // 轨道编号 = 显示顺序下标（`display_track_ids` 已排除主总线）。
    let track_index: std::collections::BTreeMap<crate::ids::EntityId, u32> = project
        .display_track_ids()
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, u32::try_from(index).unwrap_or(u32::MAX)))
        .collect();
    let mut notes = Vec::new();
    for track in project.tracks.values() {
        let Some(&index) = track_index.get(&track.id) else {
            continue;
        };
        if !track_is_audible(track, any_solo) {
            continue;
        }
        let percussion_by_path = track_is_percussion(track);
        for placement in track.clips.values() {
            if placement.muted {
                continue;
            }
            let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
                continue;
            };
            let ClipContent::Midi { notes: pool } = &entry.content else {
                continue;
            };
            let placement_start = i128::from(placement.start_tick);
            let placement_end = placement_start + i128::from(placement.duration_ticks);
            for note in pool.values() {
                if !note.triggers(project.rng_seed) {
                    continue;
                }
                let is_percussion = percussion_by_path || note.channel == Some(9);
                push_note_copies(
                    &mut notes,
                    note,
                    placement,
                    placement_start,
                    placement_end,
                    index,
                    is_percussion,
                );
            }
        }
    }
    // 身份序 ≠ 时间序：按 (起始拍, 音高, 时值, 轨道) 排成稳定顺序。
    notes.sort_by(|a, b| {
        a.start_beat
            .total_cmp(&b.start_beat)
            .then_with(|| a.midi.cmp(&b.midi))
            .then_with(|| a.duration_beats.total_cmp(&b.duration_beats))
            .then_with(|| a.track.cmp(&b.track))
    });
    notes
}

/// 一个音符在一个摆放里的全部副本（循环展开 × 连击拆分），写进 `out`。
///
/// tick 域的算术逐字照搬引擎 `project_schedules`：`step =
/// (时值 / ratchet).max(1)`；脉冲起点 = 摆放起点 + 副本起点 + 微时值 +
/// `pulse × step`，夹到 `[0, 摆放末端]`；脉冲终点 = `min(起点 + step,
/// 摆放末端)`；终点 ≤ 起点 ⇒ 跳过。
#[allow(clippy::too_many_arguments)]
fn push_note_copies(
    out: &mut Vec<NoteEvent>,
    note: &MidiNote,
    placement: &crate::project::ClipPlacement,
    placement_start: i128,
    placement_end: i128,
    track: u32,
    is_percussion: bool,
) {
    let ratchet = u64::from(note.ratchet.unwrap_or(1).clamp(1, 16));
    let micro = i128::from(note.micro_timing_ticks.unwrap_or(0));
    for (copy_start, copy_duration) in placement.note_copies(note.start_tick, note.duration_ticks) {
        let step = (copy_duration / ratchet).max(1);
        let offset = i128::from(copy_start) + micro;
        for pulse in 0..ratchet {
            let raw_start = placement_start + offset + (pulse * step) as i128;
            let start_tick = raw_start.clamp(0, placement_end).min(i128::from(u64::MAX));
            let end_tick = (start_tick + i128::from(step))
                .min(placement_end)
                .min(i128::from(u64::MAX));
            if end_tick <= start_tick {
                continue;
            }
            let start_tick = start_tick as u64;
            let end_tick = end_tick as u64;
            out.push(NoteEvent {
                start_beat: ticks_to_beats(start_tick),
                duration_beats: ticks_to_beats(end_tick - start_tick),
                midi: note.pitch,
                velocity: note.velocity,
                track,
                is_percussion,
            });
        }
    }
}

/// 速度图：0 拍条目（工程 `bpm`）+ `tempo_map` 的点（tick → 拍）。
/// 非法速度（非有限、≤ 0）丢弃。
fn project_tempo_map(project: &YebanProjectV1) -> Vec<TempoChange> {
    let mut map = Vec::with_capacity(project.tempo_map.len() + 1);
    if project.bpm.is_finite() && project.bpm > 0.0 {
        map.push(TempoChange {
            beat: 0.0,
            bpm: project.bpm as f32,
        });
    }
    for (tick, bpm) in &project.tempo_map {
        if bpm.is_finite() && *bpm > 0.0 {
            map.push(TempoChange {
                beat: ticks_to_beats(*tick),
                bpm: *bpm as f32,
            });
        }
    }
    map
}

/// 段落 Marker：按起始 tick 升序，小节号 = `floor(起始拍 / 每小节拍数) + 1`；
/// 同一小节只留最早一个段落。
fn project_markers(project: &YebanProjectV1, beats_per_bar: u32) -> Vec<SectionMarker> {
    let mut sections: Vec<_> = project.sections.values().collect();
    sections.sort_by_key(|section| (section.start_tick, section.id));
    let mut markers = Vec::with_capacity(sections.len());
    for section in sections {
        let bar = (ticks_to_beats(section.start_tick) / beats_per_bar as f32).floor() as u32 + 1;
        if markers
            .last()
            .is_some_and(|last: &SectionMarker| last.bar == bar)
        {
            continue;
        }
        markers.push(SectionMarker {
            bar,
            label: section.name.clone(),
            // 工程 `sections` 不存期望等级（持久化模型未改）：等级只活在
            // 分析请求的会话态里（见 dsp `AnalysisOptions::expected_levels`）。
            expected_level: None,
        });
    }
    markers
}
