//! `yeban_import_midi` 的实现 —— 把标准 MIDI 文件 (.mid) 从磁盘导入进空的 MIDI 片段。
//!
//! ## 需求规范 ID
//!
//! - `[MCP-TOOL-EXT-IMPORT-MIDI]`：MIDI 导入 MCP 扩展工具契约。
//! - `[ARCH-EXT-002]`：外部 DAW 与标准 MIDI 文件互操作规范。
//! - `[MODEL-AST-001]`：960 PPQ 整数时钟契约。
//! - `[MODEL-AST-005]`：`MidiNote` 核心实体与属性映射。
//!
//! ## 架构设计
//!
//! 纯映射复用 `yeban-midi` 的共享实现 [`yeban_midi::import::plan_midi_import`]。
//! 目标片段必须存在且为空；导入产生的所有音符与元数据变更在同一次提交中原子落盘，
//! 保证一次撤销即可完全还原。

use std::path::PathBuf;

use serde_json::{Map, Value, json};

use yeban_midi::import::{
    MODEL_PPQ, MidiImportError, MidiImportLosses, MidiImportOptions, MidiLyricCounts, destination,
    import_target_is_empty, plan_midi_import,
};
use yeban_midi::midi::{MidiAuxEventKind, parse_smf};
use yeban_model::project::YebanProjectV1;
use yeban_model::{ClipContent, EntityId, Op, OpOrigin};

use crate::tools::{ErrorCode, ToolResponse};
use crate::undo_session::CommitRequest;

use super::error::Fault;
use super::ids::deterministic_id;

/// 多轨拆分时新建音轨的描述信息。
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CreatedTrackInfo {
    /// 新建音轨的身份。
    pub track_id: EntityId,
    /// 新建音轨的名称。
    pub track_name: String,
    /// 放置在该轨上的新建片段身份。
    pub clip_id: EntityId,
}

/// 一次 `yeban_import_midi` 的只读规划产物。
#[derive(Debug, Clone)]
pub struct MidiImport {
    /// 源标准 MIDI 文件路径。
    pub path: PathBuf,
    /// 源文件字节长度。
    pub byte_len: u64,
    /// SMF 格式号（`0` 或 `1`）。
    pub format: u16,
    /// 源文件的时间分度（PPQ）。
    pub source_ppq: u16,
    /// 工程目标时间分度（恒为 960 PPQ）。
    pub target_ppq: u64,
    /// 目标音轨身份。
    pub track_id: EntityId,
    /// 目标片段身份。
    pub clip_id: EntityId,
    /// 导入的音符条数。
    pub note_count: usize,
    /// ⭐ T-FIX-354：导入后**实际可听**的音符条数（`note_count` 的子集）。
    pub audible_notes: usize,
    /// ⭐ T-FIX-354：导入后**一遍都排不出来**的音符条数。
    pub clipped_notes: usize,
    /// ⭐ T-FIX-354：目标摆放导入**前**的时长（tick）。
    pub target_duration_ticks: u64,
    /// ⭐ T-FIX-354：目标摆放容纳全部内容所需的**最小**时长（tick）。
    pub required_duration_ticks: u64,
    /// 歌词读取、归属与丢弃计数。
    pub lyric_counts: MidiLyricCounts,
    /// 音符与歌词以外的逐类丢失统计。
    pub losses: MidiImportLosses,
    /// 采纳到工程全局 `bpm` 的速度值（未采纳为 `None`）。
    pub adopted_tempo_bpm: Option<f64>,
    /// 采纳到工程全局 `time_signature` 的拍号 `(numerator, denominator)`（未采纳为 `None`）。
    pub adopted_time_signature: Option<(u8, u8)>,
    /// 采纳到工程全局 `tempo_map` 的变速映射点 (tick -> BPM)（未采纳为 None）。
    pub adopted_tempo_map: Option<std::collections::BTreeMap<u64, f64>>,
    /// 采纳到工程全局 `time_signature_map` 的拍号映射点 (tick -> (num, den))（未采纳为 None）。
    pub adopted_time_signature_map: Option<std::collections::BTreeMap<u64, (u8, u8)>>,
    /// 采纳到工程全局 `key_signature` 的调号 `(sharps_flats, is_minor)`（未采纳为 `None`）。
    pub adopted_key_signature: Option<(i8, bool)>,
    /// 采纳到目标音轨 `TrackV3::name` 的轨名（未采纳为 `None`）。
    pub adopted_track_name: Option<String>,
    /// 采纳到工程全局 `title` 的标题（未采纳为 `None`）。
    pub adopted_title: Option<String>,
    /// 采纳到工程全局 `author` 的版权声明（未采纳为 `None`）。
    pub adopted_copyright: Option<String>,
    /// 待原子提交的模型操作序列。
    pub ops: Vec<Op>,
    /// 多轨拆分时额外创建的音轨列表（若未拆轨或仅单轨则为空 Vec）。
    pub created_tracks: Vec<CreatedTrackInfo>,
}

fn map_import_error(err: MidiImportError) -> Fault {
    match err {
        MidiImportError::Parse(detail) => Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("MIDI 解析失败: {detail}"),
        ),
        MidiImportError::NoDestination(detail) => {
            Fault::domain(ErrorCode::TrackNotFound, format!("目标未找到: {detail}"))
        }
        MidiImportError::Empty(detail) => Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("MIDI 文件为空: {detail}"),
        ),
        MidiImportError::Refused(detail) => {
            Fault::domain(ErrorCode::Conflict, format!("提交被拒绝: {detail}"))
        }
    }
}

/// 只读规划：从磁盘读取 MIDI 文件，校验目标音轨与片段，产出编辑操作。
pub fn plan(project: &YebanProjectV1, arguments: &Map<String, Value>) -> Result<MidiImport, Fault> {
    let raw_path = arguments
        .get("path")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::InvalidParameterRange,
                "parameter 'path' must be a string",
            )
        })?;
    if raw_path.is_empty() {
        return Err(Fault::domain(
            ErrorCode::InvalidParameterRange,
            "parameter 'path' must not be empty",
        ));
    }
    let path = PathBuf::from(raw_path);

    let track_id_opt = super::parse_optional_entity_id(arguments, "trackId")?;
    let clip_id_opt = super::parse_optional_entity_id(arguments, "clipId")?;
    let split_tracks = arguments
        .get("splitTracks")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    if let Some(track_id) = track_id_opt
        && !project.tracks.contains_key(&track_id)
    {
        return Err(Fault::domain(
            ErrorCode::TrackNotFound,
            format!("track '{track_id}' not found"),
        ));
    }

    // ⭐ T-FIX-41：目标轨**一条摆放都没有**（新轨的常态）时自建落点。
    //
    // 旧行为是 `destination` 直接报 `NoDestination("音轨 x 没有任何摆放")`，于是
    // "往新轨导 MIDI"这条最自然的用法永远走不通 —— 必须先手工造一个**带音符**的
    // 片段当落点，而那个片段又必须是空的才能导入（自相矛盾）。
    //
    // ⭐ T-FIX-364：默认目标是**非空** MIDI 片段时同样自建落点 —— 默认种子工程
    // 只有一条 1 小节种子片段（4 颗种子音），直接导入必然 CONFLICT，调用方被迫
    // 先手工建空片段（还得猜 `durationTicks`，否则 `durationNotDerivable`）。
    // 显式给 `clipId` 时不触发：调用方点名的非空片段仍是 CONFLICT（⛔ 不静默
    // 改写目标）；默认目标本身是空片段时也不触发，走原路径。
    //
    // 这里先在工程副本里插入一条**空 MIDI 片段**并把它的身份显式交给 `destination`，
    // 摆放本身等读完文件、知道内容长度之后再补（时值必须覆盖整段，短了会裁剪音符）。
    // 两条 op 都前置到本批 ⇒ 与音符同一次原子提交、一次撤销。
    let resolved_track = match track_id_opt {
        Some(explicit) => Some(explicit),
        None => project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .copied(),
    };
    let mut auto_ops: Vec<Op> = Vec::new();
    let mut patched: Option<YebanProjectV1> = None;
    let mut auto_placement: Option<(EntityId, EntityId, EntityId)> = None;
    let mut auto_clip_id: Option<EntityId> = None;
    let auto_track: Option<EntityId> = if clip_id_opt.is_some() {
        None
    } else if let Some(track) = resolved_track
        && project
            .tracks
            .get(&track)
            .is_some_and(|track| track.clips.is_empty())
    {
        Some(track)
    } else {
        // 默认目标已定但不可用（非空 MIDI 片段）⇒ 在该目标所在的轨上自建落点。
        // `destination` 是纯函数：这里只读它决定的目标，不产生任何副作用。
        destination(
            project,
            &MidiImportOptions {
                track_id: track_id_opt,
                clip_id: None,
                split_tracks,
            },
        )
        .ok()
        .filter(|(_, default_clip)| {
            project.clip_pool.get(default_clip).is_some_and(
                |entry| matches!(&entry.content, ClipContent::Midi { notes } if !notes.is_empty()),
            )
        })
        .map(|(default_track, _)| default_track)
    };
    if let Some(auto_track) = auto_track {
        // 自建片段的身份必须与池里已有的身份不撞：同一文件两次导入默认目标时，
        // 第二次的默认目标是第一次自建的片段（已非空）⇒ 仍会触发自建；若沿用
        // 同一个派生身份，`AddClip` 会与池里已有的片段撞身份 —— 后缀递增，
        // 仍是确定性的（⛔ 不取墙钟/随机数）。
        let mut clip_id = deterministic_id(&format!(
            "import-midi-clip:{}:{}",
            auto_track.to_canonical_string(),
            path.to_string_lossy()
        ));
        let mut suffix = 2u32;
        while project.clip_pool.contains_key(&clip_id) {
            clip_id = deterministic_id(&format!(
                "import-midi-clip:{}:{}:{}",
                auto_track.to_canonical_string(),
                path.to_string_lossy(),
                suffix
            ));
            suffix += 1;
        }
        let placement_id = deterministic_id(&super::notes::placement_label(
            &clip_id.to_canonical_string(),
            &auto_track.to_canonical_string(),
            0,
        ));
        let entry = yeban_model::ClipPoolEntry {
            id: clip_id,
            name: path.file_stem().map_or_else(
                || "Imported MIDI".to_owned(),
                |stem| stem.to_string_lossy().into_owned(),
            ),
            content: ClipContent::Midi {
                notes: std::collections::BTreeMap::new(),
            },
        };
        let mut project_clone = project.clone();
        project_clone.clip_pool.insert(clip_id, entry.clone());
        auto_ops.push(Op::AddClip { clip: entry });
        auto_clip_id = Some(clip_id);
        auto_placement = Some((auto_track, clip_id, placement_id));
        patched = Some(project_clone);
    }
    let project_view: &YebanProjectV1 = patched.as_ref().unwrap_or(project);

    let (track_id, clip_id) = destination(
        project_view,
        &MidiImportOptions {
            // 自建落点时目标轨就是自建的那条轨（⛔ 不让 `destination` 按"第一条
            // 带 MIDI 摆放的轨"另选一条 —— 那会让回执的 `trackId` 与摆放真正
            // 所在的轨错开，`audibleNotes` 也会按错的轨去算）。
            track_id: track_id_opt.or(auto_track),
            clip_id: clip_id_opt.or(auto_clip_id),
            split_tracks,
        },
    )
    .map_err(map_import_error)?;

    let clip = project_view.clip_pool.get(&clip_id).ok_or_else(|| {
        Fault::domain(
            ErrorCode::ClipNotFound,
            format!("clip '{clip_id}' not found in clip pool"),
        )
    })?;

    match &clip.content {
        ClipContent::Midi { notes } => {
            if !notes.is_empty() {
                return Err(Fault::domain(
                    ErrorCode::Conflict,
                    format!(
                        "destination clip '{clip_id}' is not empty (contains {} notes)",
                        notes.len()
                    ),
                ));
            }
        }
        ClipContent::Audio { .. } => {
            return Err(Fault::domain(
                ErrorCode::InvalidParameterRange,
                format!("destination clip '{clip_id}' is an audio clip, not a MIDI clip"),
            ));
        }
    }

    let bytes = super::store::read_bounded(&path, 32 * 1024 * 1024, &path.display().to_string())?;

    let parsed = parse_smf(&bytes).map_err(|err| {
        Fault::domain(
            ErrorCode::InvalidParameterRange,
            format!("failed to parse MIDI file '{}': {err}", path.display()),
        )
    })?;

    // 自建摆放的时值必须覆盖整段内容：短了渲染会按摆放裁剪导入的音符。
    let mut auto_placement_value: Option<yeban_model::ClipPlacement> = None;
    if let Some((auto_track, auto_clip, placement_id)) = auto_placement {
        let span_ticks = parsed
            .notes
            .iter()
            .map(|note| note.end_tick)
            .max()
            .unwrap_or(0);
        let source_ppq = u64::from(parsed.ppq.max(1));
        let span_in_model = span_ticks
            .saturating_mul(MODEL_PPQ)
            .saturating_add(source_ppq - 1)
            / source_ppq;
        let placement = yeban_model::ClipPlacement {
            id: placement_id,
            clip_id: auto_clip,
            start_tick: 0,
            duration_ticks: span_in_model.max(MODEL_PPQ),
            ..yeban_model::ClipPlacement::default()
        };
        auto_ops.push(Op::AddClipPlacement {
            track_id: auto_track,
            placement,
        });
        auto_placement_value = Some(placement);
    }

    // ⭐ T-FIX-364：回执的可听读数必须按"导入后生效"的摆放算 —— 自建摆放是
    // 本次提交的一部分，`plan_midi_import` 必须在"已经有这条摆放"的工程视图
    // 上算 `audibleNotes`，否则自建落点永远报 `audibleNotes: 0 /
    // clippedNotes: N`（旧实现只把摆放塞进 `auto_ops`，规划时视图里还没有
    // 它）。这里只克隆视图供规划用，真正的落点仍由 `auto_ops` 里的
    // `AddClipPlacement` 一次提交创建；非自建路径视图不变，逐字同旧行为。
    // （与 GUI 的 T-FIX-720 同一做法：先补摆放再重规划。）
    let plan_view_owned: Option<YebanProjectV1> =
        match (auto_placement, auto_placement_value.as_ref()) {
            (Some((auto_track, _, _)), Some(placement)) => {
                let mut view = project_view.clone();
                view.tracks
                    .get_mut(&auto_track)
                    .expect("自建落点的轨必须在工程视图里")
                    .clips
                    .insert(placement.id, *placement);
                Some(view)
            }
            _ => None,
        };
    let plan_view: &YebanProjectV1 = plan_view_owned.as_ref().unwrap_or(project_view);

    let mut plan = plan_midi_import(
        plan_view,
        &parsed,
        &MidiImportOptions {
            track_id: Some(track_id),
            clip_id: Some(clip_id),
            split_tracks,
        },
    )
    .map_err(map_import_error)?;

    // 自建的落点必须排在音符之前：`AddNote` 依赖片段已经在池子里。
    if !auto_ops.is_empty() {
        auto_ops.append(&mut plan.ops);
        plan.ops = auto_ops;
    }

    let mut adopted_tempo_bpm = None;
    let mut adopted_time_signature = None;
    let mut adopted_tempo_map: std::collections::BTreeMap<u64, f64> =
        std::collections::BTreeMap::new();
    let mut adopted_time_signature_map: std::collections::BTreeMap<u64, (u8, u8)> =
        std::collections::BTreeMap::new();
    let mut adopted_key_signature = None;
    let mut adopted_track_name = None;
    let mut adopted_title = None;
    let mut adopted_copyright = None;

    for op in &plan.ops {
        match op {
            Op::SetTempo { new_bpm, .. } => adopted_tempo_bpm = Some(*new_bpm),
            Op::SetTempoPoint {
                tick,
                new_bpm: Some(bpm),
                ..
            } => {
                adopted_tempo_map.insert(*tick, *bpm);
            }
            Op::SetTimeSignature {
                new_time_signature, ..
            } => {
                adopted_time_signature =
                    Some((new_time_signature.numerator, new_time_signature.denominator));
            }
            Op::SetTimeSignaturePoint {
                tick,
                new_time_signature: Some(ts),
                ..
            } => {
                adopted_time_signature_map.insert(*tick, (ts.numerator, ts.denominator));
            }
            Op::SetKeySignature {
                new_key_signature, ..
            } => {
                adopted_key_signature = new_key_signature.map(|ks| (ks.sharps_flats, ks.is_minor));
            }
            Op::SetTrackName { new_name, .. } => {
                adopted_track_name = Some(new_name.clone());
            }
            Op::SetTitle { new_title, .. } => adopted_title = Some(new_title.clone()),
            Op::SetAuthor { new_author, .. } => adopted_copyright = Some(new_author.clone()),
            _ => {}
        }
    }

    if adopted_tempo_bpm.is_none()
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::Tempo)
            .is_some_and(|c| c.read > 0 && c.dropped == 0)
    {
        adopted_tempo_bpm = Some(project.bpm);
    }
    if adopted_time_signature.is_none()
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::TimeSignature)
            .is_some_and(|c| c.read > 0 && c.dropped == 0)
    {
        adopted_time_signature = Some((
            project.time_signature.numerator,
            project.time_signature.denominator,
        ));
    }
    // ⭐ [ARCH-EXT-002]：调号的采纳门槛是"目标工程无内容"（同 tempo / 拍号）。
    // ops 里没有 `SetKeySignature` 时（文件唯一 tick 0 调号与工程现值相同 ⇒ 不发空操作）
    // 回退读工程现值；非空工程会把整类记为 `dropped`，故 `dropped == 0` 已隐含目标为空。
    let target_empty = import_target_is_empty(project);
    if adopted_key_signature.is_none()
        && target_empty
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::KeySignature)
            .is_some_and(|c| c.read > 0 && c.dropped == 0)
    {
        adopted_key_signature = project
            .key_signature
            .map(|ks| (ks.sharps_flats, ks.is_minor));
    }
    // ⭐ [ARCH-EXT-002]：变速映射点的采纳门槛同调号（目标为空 + 整类零丢失）。
    // ops 里没有 `SetTempoPoint`（文件全部变速都落在 tick 0 ⇒ 只写 `project.bpm`）时，
    // 若文件读入多于一条变速且零丢失，回退读工程现值 `tempo_map`。
    if adopted_tempo_map.is_empty()
        && target_empty
        && !project.tempo_map.is_empty()
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::Tempo)
            .is_some_and(|c| c.read > 1 && c.dropped == 0)
    {
        adopted_tempo_map = project.tempo_map.clone();
    }
    if adopted_time_signature_map.is_empty()
        && target_empty
        && !project.time_signature_map.is_empty()
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::TimeSignature)
            .is_some_and(|c| c.read > 1 && c.dropped == 0)
    {
        adopted_time_signature_map = project
            .time_signature_map
            .iter()
            .map(|(t, ts)| (*t, (ts.numerator, ts.denominator)))
            .collect();
    }
    if adopted_track_name.is_none()
        && plan
            .losses
            .aux_events
            .get(&MidiAuxEventKind::TrackName)
            .is_some_and(|c| c.read > 0 && c.dropped == 0)
        && let Some(track) = project.tracks.get(&track_id)
    {
        adopted_track_name = Some(track.name.clone());
    }
    // ⚠ 标题 / 署名**没有**"回退读工程现值"这一步：两者的采纳门槛都要求工程字段**为空**
    // （不覆盖作者已写好的值），所以每次真采纳都会发 `SetTitle` / `SetAuthor`，
    // ⛔ 不存在"值相同 ⇒ 不发操作"的情形。旧版对标题按 `TrackName` 类的 `dropped == 0`
    // 回退读 `project.title` —— 那条在"目标工程已有标题、文件只有一条轨名"时会**谎报**
    // 采纳（工具契约：未采纳一律 `null`）。

    let mut created_tracks = Vec::new();
    let mut pending_track: Option<(EntityId, String)> = None;
    for op in &plan.ops {
        match op {
            Op::AddTrack { track } => {
                pending_track = Some((track.id, track.name.clone()));
            }
            Op::AddClipPlacement {
                track_id,
                placement,
            } => {
                if let Some((pending_id, name)) = pending_track.take()
                    && pending_id == *track_id
                {
                    created_tracks.push(CreatedTrackInfo {
                        track_id: *track_id,
                        track_name: name,
                        clip_id: placement.clip_id,
                    });
                }
            }
            _ => {}
        }
    }

    Ok(MidiImport {
        path,
        byte_len: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        format: plan.losses.format.number(),
        source_ppq: plan.ppq,
        target_ppq: MODEL_PPQ,
        track_id,
        clip_id,
        note_count: plan.notes,
        audible_notes: plan.audible_notes,
        clipped_notes: plan.clipped_notes,
        target_duration_ticks: plan.target_duration_ticks,
        required_duration_ticks: plan.required_duration_ticks,
        lyric_counts: plan.lyrics,
        losses: plan.losses,
        adopted_tempo_bpm,
        adopted_time_signature,
        adopted_tempo_map: (!adopted_tempo_map.is_empty()).then_some(adopted_tempo_map),
        adopted_time_signature_map: (!adopted_time_signature_map.is_empty())
            .then_some(adopted_time_signature_map),
        adopted_key_signature,
        adopted_track_name,
        adopted_title,
        adopted_copyright,
        ops: plan.ops,
        created_tracks,
    })
}

/// 把规划产物格式化为结构化 JSON 视图。
pub fn describe(import: &MidiImport) -> Value {
    let losses_val: Value = serde_json::from_str(&import.losses.to_json())
        .unwrap_or_else(|_| Value::String(import.losses.to_json()));

    json!({
        "op": "import_midi",
        "path": import.path.display().to_string(),
        "byteLen": import.byte_len,
        "format": import.format,
        "sourcePpq": import.source_ppq,
        "targetPpq": import.target_ppq,
        "trackId": import.track_id.to_canonical_string(),
        "clipId": import.clip_id.to_canonical_string(),
        "noteCount": import.note_count,
        // ⭐ T-FIX-354：`noteCount` 单独不再是"导入成功"的证据 —— 这四个读数
        // 与 GUI/CLI 回执同一份 `MidiImportPlan`（⛔ 不重算）；扩展 op 在 `ops`
        // 里与音符同一批次提交（`applied` 为 true 时已含它）。
        "audibleNotes": import.audible_notes,
        "clippedNotes": import.clipped_notes,
        "targetDurationTicks": import.target_duration_ticks,
        "requiredDurationTicks": import.required_duration_ticks,
        // 与 `MidiLyricCounts` 同口径的五桶：`read == attached + dropped`；
        // `nonUtf8` / `empty` 是**连候选都不是**的两桶（⛔ 猜文本），必须让调用方看见。
        "lyricCounts": {
            "attached": import.lyric_counts.attached,
            "orphan": import.lyric_counts.dropped,
            "dropped": import.lyric_counts.dropped,
            "read": import.lyric_counts.read,
            "nonUtf8": import.lyric_counts.non_utf8,
            "empty": import.lyric_counts.empty,
        },
        "losses": losses_val,
        "lossTotal": import.losses.loss_total(),
        "adoptedTempoBpm": import.adopted_tempo_bpm,
        "adoptedTimeSignature": import.adopted_time_signature.map(|(n, d)| json!({ "numerator": n, "denominator": d })),
        "adoptedTempoMap": import.adopted_tempo_map.as_ref().map(|m| {
            m.iter().map(|(t, bpm)| (t.to_string(), json!(*bpm))).collect::<std::collections::BTreeMap<String, Value>>()
        }),
        "adoptedTimeSignatureMap": import.adopted_time_signature_map.as_ref().map(|m| {
            m.iter().map(|(t, (num, den))| (t.to_string(), json!({ "numerator": num, "denominator": den }))).collect::<std::collections::BTreeMap<String, Value>>()
        }),
        "adoptedKeySignature": import.adopted_key_signature.map(|(sf, m)| json!({ "sharpsFlats": sf, "isMinor": m })),
        "adoptedTrackName": import.adopted_track_name,
        "adoptedTitle": import.adopted_title,
        "adoptedCopyright": import.adopted_copyright,
        "createdTracks": import.created_tracks.iter().map(|t| json!({
            "trackId": t.track_id.to_canonical_string(),
            "name": t.track_name,
            "clipId": t.clip_id.to_canonical_string(),
        })).collect::<Vec<_>>(),
    })
}

/// 执行 MIDI 导入：原子提交所有 ops 到撤销历史中。
pub fn apply(domain: &mut super::Domain, import: &MidiImport) -> Result<ToolResponse, Fault> {
    let now_ms = domain.now_ms();
    let commit_id = {
        let super::Domain {
            active,
            graph,
            undo,
            ..
        } = domain;
        let active = active.as_mut().ok_or_else(super::no_active_project)?;
        super::undo_session::commit(
            graph,
            &mut active.project,
            undo,
            CommitRequest {
                now_ms,
                origin: OpOrigin::McpEdit {
                    agent_name: super::AGENT_NAME.to_owned(),
                },
                message: format!("import_midi {}", import.path.display()),
                ops: import.ops.clone(),
            },
        )
        .map_err(super::undo_refusal_to_fault)?
    };

    let record = domain
        .graph
        .commits
        .get(&commit_id)
        .cloned()
        .ok_or_else(|| {
            Fault::domain(
                ErrorCode::EntityNotFound,
                format!("commit {commit_id} not found after commit"),
            )
        })?;

    let mut payload = describe(import);
    if let Value::Object(ref mut map) = payload {
        map.insert("commit".to_owned(), super::commit_summary(&record));
        map.insert("applied".to_owned(), Value::Bool(true));
        map.insert("commitCount".to_owned(), Value::from(domain.commit_count()));
    }
    Ok(ToolResponse::success(payload))
}
