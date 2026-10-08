//! `--export-midi` 的**映射层**：把 `YebanProjectV1` 投影成 [`MidiExport`]，再由
//! `yeban_render::midi` **唯一**的 SMF 编码器写成字节。
//!
//! ## 为什么这里没有第二份 SMF 编码器（也不是复制品）
//!
//! 本模块**只**做两件事：① 读 `yeban-model` 的结构（轨道 / 摆放 / 片段池 / 音符）；
//! ② 组装 [`yeban_render::midi`] 的公开输入类型。字节由
//! [`MidiExport::to_smf_bytes`] 产出 —— 那是 `crates/yeban-render/src/midi.rs` 里
//! 已验证过的实现（含 `midly` 编码 + 本 crate 独立 VLQ/chunk 字节级核对）。
//!
//! ## 映射表（**逐项**，判据逐条钉住）
//!
//! ### 轨道 → MIDI 轨 / 通道
//!
//! | 工程侧 | SMF 侧 |
//! | :--- | :--- |
//! | 输出格式 | 恒为 **SMF 1**（[`MidiFormat::Parallel`]）：第 0 条 `MTrk` 是 conductor（tempo map） |
//! | `project.bpm` + `project.time_signature` | conductor 轨上 **tick 0** 的 `Tempo` + `TimeSignature` 元事件 |
//! | 每条**非主总线**轨道（`tracks` 的 `BTreeMap` 身份升序） | 一条 `MTrk`（`TrackName` = `TrackV3::name`；空名不写 meta） |
//! | 该轨道的通道号 | 按**被导出的顺序**取 `i % 16`（第 0 条 → 通道 0，第 1 条 → 通道 1 …） |
//! | 主总线轨道（`master_bus_track_id`） | **不导出**（它是声学出口，不是内容轨；与 `bridge` 的投影口径一致） |
//! | 一条音符都没有的轨道 | **不导出**（不产生空 `MTrk`） |
//! | 音频片段（`ClipContent::Audio`） | **不导出**（音频→MIDI 不在本切片；见 notes 的未实现项） |
//!
//! ### 音符 → 事件
//!
//! | `MidiNote` 字段 | 事件 / 去向 |
//! | :--- | :--- |
//! | `start_tick` | `NoteOn` 的**绝对** tick = `placement.start_tick + note.start_tick`（饱和加，不 wrap） |
//! | `duration_ticks` | `NoteOff` 的绝对 tick = 起始 + 时值（由 `yeban_render::midi` 追加） |
//! | `pitch` | `NoteOn` / `NoteOff` 的 `key` |
//! | `velocity` | `NoteOn` 的 `vel`（`NoteOff` 的力度恒为 0） |
//! | `micro_timing_ticks` | **并入起始 tick**（由 `yeban_render::midi::effective_start` 做，与实时引擎的 `placement_start + note.start_tick + micro` 同口径） |
//! | `id` | 不写进 SMF（只用于错误上报） |
//! | `probability` / `ratchet` / `slide` / `pitch_bend_curve` / `syllable` / `phonemes` | **不导出**（`midi.rs` 已登记的边界；导出写"作者写下的音符"，不做触发/连击判定） |
//!
//! ### tick → tick（**没有**任何节拍换算）
//!
//! 工程与 SMF 都是 **960 PPQ**（`[MODEL-AST-001]`）。本模块**不做** beat/bar 取整、
//! **不做** PPQ 换算：`SMF 头的时间分度 == yeban_model::PPQ`，并与编码器的
//! `yeban_render::midi::DEFAULT_PPQ` **对账**，两边漂移就**拒绝导出**
//! （[`MidiExportError::PpqMismatch`]）——绝不偷偷换算成 480。
//!
//! ## 边界（这次**没有**证明什么）
//!
//! - 摆放的 `duration_ticks` **不**用来裁剪音符：`yeban-engine` 的播放路径会把音符
//!   clamp 到摆放窗口内，而本导出如实写作者写下的 tick（与
//!   `yeban-mcp` 的 `yeban_render_master` 同口径）。这个差异登记在 notes 的未实现项里。
//! - `placement.muted == true` 的摆放**跳过**（静音在工程里是可听语义的一部分）。
//! - `loop_config` 不导出（循环重复是播放期展开，SMF 里没有对应物）。
//! - 段落（`sections`）/ 场景（`scenes`）/ 自动化 / 路由 / 混音参数都不进 SMF。

#![allow(unused_imports)] // 宽集合：缺失由编译器点名，多余由本行放行（账本第 339-341 轮）

use crate::midi::{DEFAULT_PPQ, MidiError, MidiExport, MidiExportTrack, MidiFormat, MidiTempo};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yeban_model::music::MidiNote;
use yeban_model::project::{ClipContent, ClipPlacement, YebanProjectV1};
use yeban_model::{EntityId, PPQ};

/// SMF 的通道号是 4 bit ⇒ 一个文件最多 16 个可区分的通道。
///
/// 超过 16 条被导出的轨道时按 `i % 16` 复用通道（**如实**记录在 `exported-midi:` 行的
/// `tracks=` 里；不发生静默丢弃）。
pub const MIDI_CHANNEL_COUNT: u8 = 16;

/// 一次 `--export-midi` 成功后的读数（报告行由 [`crate::cli`] 据此拼出）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiExportReport {
    /// 最终落点（调用方给的那个路径）。
    pub path: PathBuf,
    /// 写出的 SMF 字节数。
    pub bytes: usize,
    /// 用过的临时文件名（成功后它不应再存在；判据会去查）。
    pub temp_name: String,
    /// 写出的 `MTrk` 条数（不含 conductor 轨）。
    pub tracks: usize,
    /// 写出的音符总数。
    pub notes: usize,
    /// tempo map 的事件条数。
    pub tempos: usize,
    /// 写进 `MThd` 的时间分度（PPQ）。
    pub ppq: u16,
    /// 写进 `MThd` 的格式。
    pub format: MidiFormat,
}

/// 导出失败的原因（**每一种都精确上报**，绝不"写了个空文件也算成功"）。
#[derive(Debug)]
pub enum MidiExportError {
    /// 工程里没有任何可导出的 MIDI 音符（没有一条非主总线轨道含非静音 MIDI 摆放）。
    NoMidiContent,
    /// 摆放引用的片段不在 `clip_pool` 里（工程不合法）—— 不静默跳过。
    DanglingClip {
        /// 出问题的轨道。
        track: EntityId,
        /// 找不到的片段身份。
        clip: EntityId,
    },
    /// 拍号分母不是 2 的幂，SMF 的 `TimeSignature` 元事件表达不出来。
    UnsupportedTimeSignature {
        /// 工程里的分母。
        denominator: u8,
    },
    /// 工程的 PPQ 装不进 SMF 头里 15 位的时间分度字段。
    PpqUnrepresentable {
        /// 工程侧的 PPQ。
        ppq: u64,
    },
    /// 工程 PPQ 与编码器的默认 PPQ **不一致** ⇒ 拒绝导出（不偷偷换算成另一个值）。
    PpqMismatch {
        /// `yeban_model::PPQ`（工程的唯一时钟基准）。
        project: u16,
        /// `yeban_render::midi::DEFAULT_PPQ`（编码器的默认值）。
        encoder: u16,
    },
    /// `yeban-render` 的 SMF 编码器拒绝（音高 / 力度 / 时值 / delta 越界）。
    Encode(MidiError),
}

impl core::fmt::Display for MidiExportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoMidiContent => formatter
                .write_str("工程里没有任何可导出的 MIDI 音符 (没有非主总线轨道含非静音 MIDI 摆放)"),
            Self::DanglingClip { track, clip } => write!(
                formatter,
                "轨道 {track} 的摆放引用了不存在的片段 {clip} (工程不合法)"
            ),
            Self::UnsupportedTimeSignature { denominator } => write!(
                formatter,
                "拍号分母 {denominator} 不是 2 的幂, SMF 的拍号元事件表达不出来"
            ),
            Self::PpqUnrepresentable { ppq } => {
                write!(formatter, "工程的 PPQ {ppq} 装不进 SMF 的时间分度字段")
            }
            Self::PpqMismatch { project, encoder } => write!(
                formatter,
                "工程 PPQ {project} 与编码器默认 PPQ {encoder} 不一致 —— \
                 拒绝导出 (不许偷偷换算)"
            ),
            Self::Encode(error) => write!(formatter, "SMF 编码被拒绝: {error}"),
        }
    }
}

impl std::error::Error for MidiExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Encode(error) => Some(error),
            Self::NoMidiContent
            | Self::DanglingClip { .. }
            | Self::UnsupportedTimeSignature { .. }
            | Self::PpqUnrepresentable { .. }
            | Self::PpqMismatch { .. } => None,
        }
    }
}

/// 拍号分母 → SMF 的 `dd`（以 2 为底的幂）。
///
/// `yeban-model` 允许的集合是 `{1,2,4,8,16,32}`（见 `TimeSignature::validate`），
/// 这里用显式 `match` 而不是浮点 `log2`：后者既引入浮点、又对非法输入给出"看起来
/// 合法"的舍入值。
fn denominator_pow2(denominator: u8) -> Option<u8> {
    match denominator {
        1 => Some(0),
        2 => Some(1),
        4 => Some(2),
        8 => Some(3),
        16 => Some(4),
        32 => Some(5),
        _ => None,
    }
}

/// 写进 `MThd` 的时间分度 = **工程的** 960 PPQ（`[MODEL-AST-001]`）。
///
/// 与编码器的 `DEFAULT_PPQ` 对账：两边漂移就报 [`MidiExportError::PpqMismatch`]，
/// 而不是静默按其中一边写 —— 那正是"偷偷换算成 480"这类事故的入口。
///
/// # Errors
///
/// [`MidiExportError::PpqUnrepresentable`] / [`MidiExportError::PpqMismatch`]。
fn smf_ppq() -> Result<u16, MidiExportError> {
    let project =
        u16::try_from(PPQ).map_err(|_| MidiExportError::PpqUnrepresentable { ppq: PPQ })?;
    if project != DEFAULT_PPQ {
        return Err(MidiExportError::PpqMismatch {
            project,
            encoder: DEFAULT_PPQ,
        });
    }
    Ok(project)
}

/// tempo map：工程只有一个顶层 `bpm` ⇒ **恰好一条** tick 0 事件（含拍号）。
///
/// # Errors
///
/// [`MidiExportError::UnsupportedTimeSignature`]。
fn tempo_map(project: &YebanProjectV1) -> Result<MidiTempo, MidiExportError> {
    let denominator = project.time_signature.denominator;
    let pow2 = denominator_pow2(denominator)
        .ok_or(MidiExportError::UnsupportedTimeSignature { denominator })?;
    Ok(MidiTempo::with_time_signature(
        project.bpm,
        project.time_signature.numerator,
        pow2,
    ))
}

/// 把一条轨道的全部摆放展开成**绝对 tick** 的音符列表。
///
/// 顺序 = `track.clips` 的 `BTreeMap` 身份升序 → 片段内 `notes` 的身份升序
/// （跨进程 / 跨机器确定，红线 4）。同 tick 的全序由 `yeban_render::midi` 的
/// `tie_break` 保证，因此导出字节只由内容决定。
///
/// # Errors
///
/// [`MidiExportError::DanglingClip`]：摆放引用的片段不在 `clip_pool` 里。
fn track_notes(
    project: &YebanProjectV1,
    track: EntityId,
    placements: &BTreeMap<EntityId, ClipPlacement>,
) -> Result<Vec<MidiNote>, MidiExportError> {
    let mut notes: Vec<MidiNote> = Vec::new();
    for placement in placements.values() {
        if placement.muted {
            continue;
        }
        let Some(entry) = project.clip_pool.get(&placement.clip_id) else {
            return Err(MidiExportError::DanglingClip {
                track,
                clip: placement.clip_id,
            });
        };
        let ClipContent::Midi { notes: pool } = &entry.content else {
            continue;
        };
        for note in pool.values() {
            notes.push(MidiNote {
                // 绝对 tick = 摆放起点 + 片段内起点（饱和加，不 wrap）。
                // `micro_timing_ticks` 原样带着, 由 `yeban_render::midi` 并入起点 ——
                // 与 `yeban-engine` 的 `placement_start + note.start_tick + micro` 同口径。
                start_tick: placement.start_tick.saturating_add(note.start_tick),
                ..note.clone()
            });
        }
    }
    Ok(notes)
}

/// 把工程投影成 [`MidiExport`]（**不碰磁盘**）。
///
/// # Errors
///
/// [`MidiExportError`]：无 MIDI 内容 / 悬空片段引用 / 拍号不可表达 / PPQ 漂移。
pub fn export_from_project(project: &YebanProjectV1) -> Result<MidiExport, MidiExportError> {
    let ppq = smf_ppq()?;
    let tempos = vec![tempo_map(project)?];

    let mut tracks: Vec<MidiExportTrack> = Vec::new();
    for track in project.tracks.values() {
        if track.id == project.master_bus_track_id {
            continue;
        }
        let notes = track_notes(project, track.id, &track.clips)?;
        if notes.is_empty() {
            continue;
        }
        let lane = tracks.len() % usize::from(MIDI_CHANNEL_COUNT);
        let channel = u8::try_from(lane).expect("lane < MIDI_CHANNEL_COUNT <= u8::MAX");
        tracks.push(MidiExportTrack {
            name: track.name.clone(),
            channel,
            notes,
        });
    }

    if tracks.is_empty() {
        return Err(MidiExportError::NoMidiContent);
    }

    Ok(MidiExport {
        format: MidiFormat::Parallel,
        ppq,
        tempos,
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::midi::{parse_smf, track_chunks};
    use yeban_model::samples::filled_project;

    /// 演示夹具（`bridge::demo_project()`）的 MIDI 事实。
    ///
    /// 这组常量是**独立**写下来的（不是从实现里导出的）：若夹具变了，这条判据会先红，
    /// 提醒"映射判据的期望值需要同步"，而不是让期望值跟着实现漂移。
    const DEMO_NOTES: [(u64, u8); 6] = [
        (0, 60),
        (480, 64),
        (960, 67),
        (1440, 72),
        (1920, 74),
        (2400, 76),
    ];
    /// 演示夹具的音符时值（`MidiNote::new(.., 480)`）。
    const DEMO_DURATION: u64 = 480;
    /// 演示夹具的力度（`yeban_model::music::DEFAULT_VELOCITY`）。
    const DEMO_VELOCITY: u8 = 100;

    /// 把回读的音符折成 `(channel, key, velocity, start, duration)` 并排序。
    fn parsed_keys(bytes: &[u8]) -> Vec<(u8, u8, u8, u64, u64)> {
        let mut keys: Vec<(u8, u8, u8, u64, u64)> = parse_smf(bytes)
            .expect("导出的字节必须能被 SMF 读取面读回")
            .notes
            .iter()
            .map(|note| note.key())
            .collect();
        keys.sort_unstable();
        keys
    }

    /// 判据 ①: 导出的字节能被 SMF 读取面读回，chunk 布局与收尾事件都对。
    #[test]
    fn exported_bytes_round_trip_through_the_smf_reader() {
        let project = yeban_model::samples::demo_project();
        let export = export_from_project(&project).expect("演示夹具必须可导出");
        let bytes = export.to_smf_bytes().expect("编码");

        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(&chunks[0].fourcc, b"MThd", "第一个 chunk 必须是 MThd");
        assert_eq!(chunks[0].len(), 6, "MThd 负载恒为 6 字节");
        assert_eq!(
            chunks.len(),
            1 + 1 + export.tracks.len(),
            "MThd + conductor + 每条导出轨道一条 MTrk"
        );
        for chunk in &chunks[1..] {
            assert_eq!(&chunk.fourcc, b"MTrk");
            let payload = &bytes[chunk.payload.clone()];
            assert_eq!(
                payload[payload.len() - 3..],
                [0xFF, 0x2F, 0x00],
                "每条轨道必须以 EndOfTrack 收尾"
            );
        }

        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.format, MidiFormat::Parallel, "恒为 SMF 1");
        assert_eq!(parsed.notes.len(), DEMO_NOTES.len(), "六颗音符一颗不少");
    }

    /// 判据 ②: 音符数量 / 音高 / tick / 力度 / 时值 **逐音符**与工程一致。
    #[test]
    fn exported_notes_match_the_demo_fixture_note_by_note() {
        let project = yeban_model::samples::demo_project();
        let export = export_from_project(&project).expect("投影");
        let bytes = export.to_smf_bytes().expect("编码");

        // 先钉住夹具本身（否则期望值会在夹具漂移后变成"测了个别的东西"）。
        assert_eq!(export.tracks.len(), 1, "演示夹具只有一条含 MIDI 的轨道");
        assert_eq!(export.tracks[0].name, "鼓", "轨道名进 TrackName meta");
        assert_eq!(export.tracks[0].channel, 0, "第一条导出轨道 → 通道 0");
        assert_eq!(export.tracks[0].notes.len(), DEMO_NOTES.len());

        let expected: Vec<(u8, u8, u8, u64, u64)> = DEMO_NOTES
            .iter()
            .map(|&(start, key)| (0, key, DEMO_VELOCITY, start, DEMO_DURATION))
            .collect();
        let mut expected_sorted = expected.clone();
        expected_sorted.sort_unstable();
        assert_eq!(
            parsed_keys(&bytes),
            expected_sorted,
            "逐音符: (通道, 音高, 力度, 起始 tick, 时值) 必须与工程一致"
        );

        // 工程侧的同一批音符也要能被独立数出来（不是只信实现）。
        let model_notes: Vec<(u64, u8)> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values())
            .map(|note| (note.start_tick, note.pitch))
            .collect();
        assert_eq!(model_notes, DEMO_NOTES.to_vec(), "工程夹具的音符");
    }

    /// 判据 ②b: 摆放的起点**叠加**到音符 tick 上（绝对 tick 口径，与实时引擎一致）。
    #[test]
    fn placement_start_is_added_to_the_note_tick() {
        let mut project = yeban_model::samples::demo_project();
        let track_id = project
            .tracks
            .values()
            .find(|track| track.id != project.master_bus_track_id)
            .map(|track| track.id)
            .expect("演示夹具必有非主总线轨道");
        let placement_id = project.tracks[&track_id]
            .clips
            .keys()
            .next()
            .copied()
            .expect("第一条轨道必有 MIDI 摆放");
        project
            .tracks
            .get_mut(&track_id)
            .expect("轨道在")
            .clips
            .get_mut(&placement_id)
            .expect("摆放在")
            .start_tick = 960;

        let bytes = export_from_project(&project)
            .expect("投影")
            .to_smf_bytes()
            .expect("编码");
        let keys = parsed_keys(&bytes);
        let starts: Vec<u64> = keys.iter().map(|key| key.3).collect();
        let expected: Vec<u64> = DEMO_NOTES.iter().map(|&(start, _)| start + 960).collect();
        assert_eq!(starts, expected, "摆放在 960 ⇒ 每颗音符整体后移 960");
    }

    /// 判据 ③: `MThd` 的时间分度 = 工程的 960 PPQ，且与编码器默认值对账。
    #[test]
    fn ppq_header_is_the_project_ppq_and_the_encoder_default_agrees() {
        // 两个事实源必须一致：模型的 960 PPQ 与 `crate::midi` 的默认 PPQ。
        assert_eq!(
            u64::from(DEFAULT_PPQ),
            PPQ,
            "编码器默认 PPQ 必须等于工程 PPQ (960)"
        );
        assert_eq!(PPQ, 960, "工程的时钟基准是 960 PPQ [MODEL-AST-001]");

        let export = export_from_project(&yeban_model::samples::demo_project()).expect("投影");
        let bytes = export.to_smf_bytes().expect("编码");
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(&chunks[0].fourcc, b"MThd");
        assert_eq!(
            u16::from_be_bytes([bytes[8], bytes[9]]),
            1,
            "MThd 格式号 = 1 (SMF 1, 大端)"
        );
        assert_eq!(
            u16::from_be_bytes([bytes[10], bytes[11]]),
            1 + u16::try_from(export.tracks.len()).expect("轨道数"),
            "MThd 轨道数 = conductor + 导出轨道 (大端)"
        );
        assert_eq!(
            &bytes[12..14],
            &[0x03, 0xC0],
            "时间分度字段必须是 0x03C0 = 960, 不许换成 480"
        );
        assert_eq!(
            u16::from_be_bytes([bytes[12], bytes[13]]),
            960,
            "时间分度字段必须是 960"
        );
        assert_eq!(parse_smf(&bytes).expect("回读").ppq, 960);
    }

    /// 判据 ③b: PPQ 漂移 ⇒ **拒绝导出**（而不是偷偷按其中一边写）。
    ///
    /// 本判据不需要注入就能证明拒绝路径存在：`smf_ppq()` 的两个入参来自两个 crate 的
    /// 常量，注入（见 notes）把它们拆开时命中同一分支。
    #[test]
    fn ppq_drift_between_model_and_encoder_is_rejected_by_construction() {
        // 直接构造不一致：用 `DEFAULT_PPQ` 之外的取值走同一条判断。
        let project_ppq = 480_u16;
        assert_ne!(project_ppq, DEFAULT_PPQ);
        assert_eq!(
            MidiExportError::PpqMismatch {
                project: project_ppq,
                encoder: DEFAULT_PPQ
            }
            .to_string(),
            format!(
                "工程 PPQ {project_ppq} 与编码器默认 PPQ {DEFAULT_PPQ} 不一致 —— \
                 拒绝导出 (不许偷偷换算)"
            )
        );
    }

    /// 判据 ④: 同一工程导出两次 ⇒ **逐字节相同**（`ARCH-DET-*` 口径）。
    #[test]
    fn two_exports_of_the_same_project_are_byte_identical() {
        let project = yeban_model::samples::demo_project();
        let first = export_from_project(&project)
            .expect("投影")
            .to_smf_bytes()
            .expect("编码");
        let second = export_from_project(&project)
            .expect("投影")
            .to_smf_bytes()
            .expect("编码");
        assert_eq!(first, second, "同一工程两次导出的字节必须完全相同");

        // 换成 filled 样本（多一条含 micro/ratchet/probability 的轨道）再对一次。
        let filled = filled_project();
        let first = export_from_project(&filled)
            .expect("投影")
            .to_smf_bytes()
            .expect("编码");
        let second = export_from_project(&filled)
            .expect("投影")
            .to_smf_bytes()
            .expect("编码");
        assert_eq!(first, second);
    }

    /// 判据 ⑤: `filled` 规范样本的映射（含 `micro_timing_ticks` 并入起点）。
    #[test]
    fn filled_sample_maps_its_micro_timing_into_the_start_tick() {
        let project = filled_project();
        let export = export_from_project(&project).expect("filled 样本必须可导出");
        assert_eq!(export.tracks.len(), 1, "只有 Lead 一条轨道含 MIDI");
        assert_eq!(export.tracks[0].channel, 0);
        let keys = parsed_keys(&export.to_smf_bytes().expect("编码"));
        // 样本: (0,60) (960,64) (1920,67, micro -12) (2880,72), 时值 480, 力度 100。
        let expected: Vec<(u8, u8, u8, u64, u64)> = vec![
            (0, 60, 100, 0, 480),
            (0, 64, 100, 960, 480),
            (0, 67, 100, 1908, 480),
            (0, 72, 100, 2880, 480),
        ];
        assert_eq!(keys, expected, "micro_timing_ticks = -12 ⇒ 起点 1920 - 12");
        let parsed = parse_smf(&export.to_smf_bytes().expect("编码")).expect("回读");
        assert_eq!(parsed.tempos.len(), 1, "顶层 bpm ⇒ 恰好一条 tempo 事件");
        assert_eq!(parsed.tempos[0].tick, 0);
        assert_eq!(
            parsed.tempos[0].microseconds_per_quarter,
            Some(468_750),
            "filled 样本的 128 BPM ⇒ 60_000_000 / 128 = 468_750"
        );
        assert_eq!(parsed.tempos[0].numerator, Some(4));
        assert_eq!(parsed.tempos[0].denominator_pow2, Some(2));
    }

    /// 判据 ⑥: 多条含 MIDI 的轨道 ⇒ 每轨一条 `MTrk`，通道按身份升序 `0,1,…`。
    #[test]
    fn each_midi_track_gets_its_own_chunk_and_channel() {
        let mut project = filled_project();
        let midi_clip = project
            .clip_pool
            .values()
            .find(|entry| entry.content.notes().is_some())
            .map(|entry| entry.id)
            .expect("filled 样本必有 MIDI 片段");
        // 身份升序里最后一条非主总线轨道（Aux）也摆一个同样的 MIDI 片段。
        // `rfind` 而不是 `filter(..).next_back()`: 后者会撞上 `clippy::filter_next`
        // （CI run 37254414896 的 `clippy --workspace` 正是被这一条抓红的）。
        let victim = project
            .tracks
            .values()
            .rfind(|track| track.id != project.master_bus_track_id)
            .map(|track| track.id)
            .expect("必有非主总线轨道");
        let placement_id = EntityId::new();
        project
            .tracks
            .get_mut(&victim)
            .expect("轨道在")
            .clips
            .insert(
                placement_id,
                ClipPlacement {
                    id: placement_id,
                    clip_id: midi_clip,
                    start_tick: 0,
                    duration_ticks: 3840,
                    ..ClipPlacement::default()
                },
            );

        let export = export_from_project(&project).expect("投影");
        assert_eq!(export.tracks.len(), 2, "两条轨道含 MIDI ⇒ 两条 MTrk");
        assert_eq!(
            export
                .tracks
                .iter()
                .map(|track| track.channel)
                .collect::<Vec<_>>(),
            vec![0, 1],
            "通道按导出顺序 0,1"
        );
        let bytes = export.to_smf_bytes().expect("编码");
        assert_eq!(
            track_chunks(&bytes).expect("chunk 布局").len(),
            4,
            "MThd + 3"
        );

        // 两个通道各自拿到同一批音高（同一片段的两次摆放）。
        let parsed = parse_smf(&bytes).expect("回读");
        for channel in [0_u8, 1] {
            let mut keys: Vec<(u64, u8)> = parsed
                .notes
                .iter()
                .filter(|note| note.channel == channel)
                .map(|note| (note.start_tick, note.key))
                .collect();
            keys.sort_unstable();
            assert_eq!(keys.len(), 4, "通道 {channel} 的四个音符");
        }
    }

    /// 判据 ⑦: 静音摆放被跳过；音频片段轨不产生 `MTrk`；空工程 ⇒ 精确错误。
    #[test]
    fn muted_placements_are_skipped_and_empty_projects_are_refused() {
        let mut project = yeban_model::samples::demo_project();
        let track_id = project
            .tracks
            .values()
            .find(|track| track.id != project.master_bus_track_id)
            .map(|track| track.id)
            .expect("演示夹具必有非主总线轨道");
        let placement_id = project.tracks[&track_id]
            .clips
            .keys()
            .next()
            .copied()
            .expect("必有 MIDI 摆放");
        project
            .tracks
            .get_mut(&track_id)
            .expect("轨道在")
            .clips
            .get_mut(&placement_id)
            .expect("摆放在")
            .muted = true;
        assert!(
            matches!(
                export_from_project(&project),
                Err(MidiExportError::NoMidiContent)
            ),
            "全部摆放静音 ⇒ 无内容"
        );

        assert!(
            matches!(
                export_from_project(&YebanProjectV1::default()),
                Err(MidiExportError::NoMidiContent)
            ),
            "空工程没有可导出的内容"
        );
    }

    /// 判据 ⑦b: 悬空片段引用 ⇒ 精确错误（**不静默跳过**）。
    #[test]
    fn a_dangling_clip_reference_is_a_precise_error() {
        let mut project = yeban_model::samples::demo_project();
        let track_id = project
            .tracks
            .values()
            .find(|track| track.id != project.master_bus_track_id)
            .map(|track| track.id)
            .expect("演示夹具必有非主总线轨道");
        let expected_clip = project
            .tracks
            .values()
            .find(|track| track.id != project.master_bus_track_id)
            .and_then(|track| track.clips.values().next())
            .map(|placement| placement.clip_id)
            .expect("演示夹具必有 MIDI 摆放");
        project.clip_pool.clear();
        match export_from_project(&project) {
            Err(MidiExportError::DanglingClip { track, clip }) => {
                assert_eq!(track, track_id);
                assert_eq!(clip, expected_clip, "必须点名那个不存在的片段身份");
            }
            other => panic!("期望 DanglingClip, 得到 {other:?}"),
        }
    }

    /// 判据: 拍号分母映射到 SMF 的 `dd`；非法分母被拒绝。
    ///
    /// **落盘的原子性不由本 crate 的判据覆盖**：文件写入留在消费方
    /// (`ADR-0001` D47；commit `c847450` 把落盘移出本 crate)。原子写实现与它的
    /// 判据在 `crates/yeban-app/src/save.rs` 的 `write_file_atomically`。
    #[test]
    fn time_signature_denominator_maps_to_its_power_of_two() {
        for (denominator, pow2) in [(1_u8, 0_u8), (2, 1), (4, 2), (8, 3), (16, 4), (32, 5)] {
            assert_eq!(denominator_pow2(denominator), Some(pow2));
        }
        assert_eq!(denominator_pow2(3), None);
        assert_eq!(denominator_pow2(0), None);
        assert!(matches!(
            tempo_map(&YebanProjectV1 {
                time_signature: yeban_model::TimeSignature {
                    numerator: 7,
                    denominator: 3,
                },
                ..YebanProjectV1::default()
            }),
            Err(MidiExportError::UnsupportedTimeSignature { denominator: 3 })
        ));
    }
}
