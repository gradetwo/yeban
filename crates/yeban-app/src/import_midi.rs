//! MIDI 文件**导入**：把一段 SMF 里的音符变成工程里的**真实编辑操作**，并如实标注来源。
//!
//! ## 为什么落在这里（⛔ 零新增依赖边）
//!
//! `yeban-midi` **不依赖** `yeban-model`，`yeban-model` **不依赖** `yeban-midi`；
//! 而 `yeban-app` 是**唯一**同时依赖两者的 crate ⇒ "文件里的音符 ⇒ 工程操作"这层映射
//! 只能落在 app，且**不需要**任何新依赖边。
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! bytes ──▶ yeban_midi::midi::parse_smf（既有解析器）
//!       ──▶ ParsedNote ⇒ MidiNote（PPQ 换算到模型的 960；绝对 tick 口径与铅笔一致）
//!       ──▶ Op::AddNote{track_id, clip_id, note}
//!       ──▶ **一次** UndoPort::commit_ops_with_origin(now_ms, OpOrigin::MidiInput, …, ops)
//! ```
//!
//! ## 来源标记是**本切片的核心语义**
//!
//! 每个音符的来源都必须是 [`yeban_model::OpOrigin::MidiInput`]（⛔ 不是 `UserUi`）：
//! `UndoPort::commit_ops` 把来源写死成 `UserUi`，所以这里走**显式来源**的入口，
//! 并把来源打进报告行（`origin=midi-input`）⇒ 判据可以**读回**它。
//!
//! ## 全应用或全不应用
//!
//! 解析与映射**全部**在提交之前完成（纯函数 `plan_midi_import`）；只有全部成功才发出**一次**
//! 提交请求 ⇒ 非法输入不会留下半个工程（与 `--export-wav` 的"不留半成品"同族）。

use std::path::{Path, PathBuf};

use yeban_midi::midi::{ParsedMidi, parse_smf};
use yeban_model::{EntityId, MidiNote, Op, OpOrigin, YebanProjectV1};

use crate::undo::UndoPort;

/// 模型的每四分音符 tick 数（960 PPQ）。导入时把文件的 PPQ 换算到它。
pub const MODEL_PPQ: u64 = 960;

/// 导入选项。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MidiImportOptions {
    /// 目标音轨；`None` = 第一条**非 master** 的音轨。
    pub track_id: Option<EntityId>,
    /// 目标片段；`None` = 目标音轨的**第一个**摆放（`placement`）。
    pub clip_id: Option<EntityId>,
}

/// 导入计划（**纯函数产物**：提交之前的一切）。
#[derive(Clone, Debug, PartialEq)]
pub struct MidiImportPlan {
    /// 待提交的操作（每个音符一个 `Op::AddNote`）。
    pub ops: Vec<Op>,
    /// 音符条数（= `ops.len()`）。
    pub notes: usize,
    /// 目标音轨。
    pub track_id: EntityId,
    /// 目标片段。
    pub clip_id: EntityId,
    /// 文件的时间分度（PPQ）。
    pub ppq: u16,
}

/// 导入报告（形态照 `MidiExportReport`：路径／条数／目标／提交身份／**来源**）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiImportReport {
    /// 来源文件路径（从字节导入时为 `None`）。
    pub path: Option<PathBuf>,
    /// 导入的音符条数。
    pub notes: usize,
    /// 目标音轨。
    pub track_id: EntityId,
    /// 目标片段。
    pub clip_id: EntityId,
    /// 本次提交的身份（撤销图里的提交 id）。
    pub commit: EntityId,
    /// 本次编辑的**来源标记**（本切片恒为 [`OpOrigin::MidiInput`]）。
    pub origin: OpOrigin,
}

impl MidiImportReport {
    /// 一行可读摘要（CLI 打印它；`origin` 用可检索的短名）。
    #[must_use]
    pub fn summary(&self) -> String {
        let origin = match self.origin {
            OpOrigin::MidiInput => "midi-input",
            OpOrigin::UserUi => "user-ui",
            _ => "other",
        };
        format!(
            "imported-midi: path={} notes={} track={} clip={} commit={} origin={origin}",
            self.path
                .as_ref()
                .map_or_else(|| "<bytes>".to_owned(), |path| path.display().to_string()),
            self.notes,
            self.track_id.to_canonical_string(),
            self.clip_id.to_canonical_string(),
            self.commit.to_canonical_string(),
        )
    }
}

/// 导入失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiImportError {
    /// SMF 解析失败（非法字节 / 截断 / 不支持的特性）。
    Parse(String),
    /// 工程里没有可落音符的目标（没有非 master 的音轨，或该音轨没有摆放）。
    NoDestination(String),
    /// 文件里没有音符（拒绝空导入，⛔ 不静默提交一个空批次）。
    Empty(String),
    /// 会话/权威拒绝提交。
    Refused(String),
}

impl core::fmt::Display for MidiImportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Parse(reason) => write!(formatter, "解析 MIDI 失败: {reason}"),
            Self::NoDestination(reason) => write!(formatter, "没有可导入的目标: {reason}"),
            Self::Empty(reason) => write!(formatter, "文件里没有音符: {reason}"),
            Self::Refused(reason) => write!(formatter, "提交被拒绝: {reason}"),
        }
    }
}

impl core::error::Error for MidiImportError {}

/// 把文件的 PPQ 换算到模型的 960 PPQ（整数运算，⛔ 不引入浮点漂移）。
#[must_use]
pub fn scale_ticks(tick: u64, ppq: u16) -> u64 {
    let ppq = u64::from(ppq.max(1));
    tick.saturating_mul(MODEL_PPQ) / ppq
}

/// **纯映射**：`ParsedNote` ⇒ `Op::AddNote`（⛔ 不改工程、⛔ 不提交）。
///
/// # Errors
///
/// [`MidiImportError::Empty`]（文件里没有音符）、[`MidiImportError::NoDestination`]（没有目标）。
pub fn plan_midi_import(
    project: &YebanProjectV1,
    parsed: &ParsedMidi,
    options: &MidiImportOptions,
) -> Result<MidiImportPlan, MidiImportError> {
    if parsed.notes.is_empty() {
        return Err(MidiImportError::Empty("解析出的音符数为 0".to_owned()));
    }
    let (track_id, clip_id) = destination(project, options)?;
    // 预分配：音符条数在解析后就已知（⛔ 不让 `Vec` 为 1000 个 Op 反复扩容）。
    let mut ops: Vec<Op> = Vec::with_capacity(parsed.notes.len());
    ops.extend(parsed.notes.iter().map(|note| {
        let start = scale_ticks(note.start_tick, parsed.ppq);
        let end = scale_ticks(note.end_tick.max(note.start_tick + 1), parsed.ppq);
        let mut model_note = MidiNote::new(
            EntityId::new(),
            start,
            note.key,
            end.saturating_sub(start).max(1),
        );
        model_note.velocity = note.velocity;
        Op::AddNote {
            track_id,
            clip_id,
            note: model_note,
        }
    }));
    let notes = ops.len();
    Ok(MidiImportPlan {
        ops,
        notes,
        track_id,
        clip_id,
        ppq: parsed.ppq,
    })
}

/// 选目标：第一条非 master 的音轨 ＋ 它的第一个摆放。
fn destination(
    project: &YebanProjectV1,
    options: &MidiImportOptions,
) -> Result<(EntityId, EntityId), MidiImportError> {
    let track_id = match options.track_id {
        Some(explicit) => explicit,
        None => *project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .ok_or_else(|| {
                MidiImportError::NoDestination("工程里没有非 master 的音轨".to_owned())
            })?,
    };
    let track = project.tracks.get(&track_id).ok_or_else(|| {
        MidiImportError::NoDestination(format!(
            "音轨 `{}` 不在工程里",
            track_id.to_canonical_string()
        ))
    })?;
    let clip_id = match options.clip_id {
        Some(explicit) => explicit,
        None => track
            .clips
            .values()
            .next()
            .map(|placement| placement.clip_id)
            .ok_or_else(|| {
                MidiImportError::NoDestination(format!(
                    "音轨 `{}` 没有任何摆放",
                    track_id.to_canonical_string()
                ))
            })?,
    };
    Ok((track_id, clip_id))
}

/// 从**字节**导入（核心路径）：解析 ⇒ 映射 ⇒ **一次**带来源的提交。
///
/// # Errors
///
/// 解析失败 / 没有目标 / 空文件 / 提交被拒绝。
pub fn import_midi_bytes(
    port: &UndoPort,
    project: &YebanProjectV1,
    bytes: &[u8],
    options: &MidiImportOptions,
    now_ms: u64,
) -> Result<MidiImportReport, MidiImportError> {
    let parsed = parse_smf(bytes).map_err(|error| MidiImportError::Parse(error.to_string()))?;
    let plan = plan_midi_import(project, &parsed, options)?;
    let commit = port
        .commit_ops_with_origin(now_ms, OpOrigin::MidiInput, "import midi", plan.ops)
        .map_err(|refusal| MidiImportError::Refused(refusal.to_string()))?;
    Ok(MidiImportReport {
        path: None,
        notes: plan.notes,
        track_id: plan.track_id,
        clip_id: plan.clip_id,
        commit,
        origin: OpOrigin::MidiInput,
    })
}

/// 从**文件**导入（CLI 路径）。
///
/// # Errors
///
/// 读文件失败（映射为 [`MidiImportError::Parse`]）＋ [`import_midi_bytes`] 的全部失败。
pub fn import_midi_file(
    port: &UndoPort,
    project: &YebanProjectV1,
    path: &Path,
    options: &MidiImportOptions,
    now_ms: u64,
) -> Result<MidiImportReport, MidiImportError> {
    let bytes = std::fs::read(path).map_err(|error| {
        MidiImportError::Parse(format!("读 `{}` 失败: {error}", path.display()))
    })?;
    let mut report = import_midi_bytes(port, project, &bytes, options, now_ms)?;
    report.path = Some(path.to_path_buf());
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yeban_midi::midi::{MidiFormat, MidiTempo, ParsedNote};

    /// 造一份**直接可用的** `ParsedMidi`（⛔ 不需要真的编码 SMF：本判据测的是**映射**）。
    fn parsed(ppq: u16, notes: Vec<ParsedNote>) -> ParsedMidi {
        ParsedMidi {
            format: MidiFormat::SingleTrack,
            ppq,
            tempos: Vec::<MidiTempo>::new(),
            notes,
        }
    }

    fn note(key: u8, velocity: u8, start: u64, end: u64) -> ParsedNote {
        ParsedNote {
            channel: 0,
            key,
            velocity,
            start_tick: start,
            end_tick: end,
        }
    }

    /// 判据 a（映射）：`ParsedNote ⇒ Op::AddNote`，PPQ 换算、音高/力度/时值逐项一致，
    /// 且目标是**第一条非 master 音轨的第一个摆放**。
    #[test]
    fn the_mapping_scales_ticks_and_keeps_pitch_velocity_and_duration() {
        let project = yeban_model::samples::filled_project();
        // 480 PPQ 的文件 ⇒ 模型 960 PPQ ⇒ 每个 tick 乘 2。
        let plan = plan_midi_import(
            &project,
            &parsed(480, vec![note(60, 100, 480, 960), note(64, 64, 0, 240)]),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(plan.ppq, 480);
        assert_eq!(plan.notes, 2);
        assert_eq!(
            plan.track_id,
            *project
                .tracks
                .keys()
                .find(|id| **id != project.master_bus_track_id)
                .expect("有一条非 master 音轨")
        );
        let mut mapped: Vec<(u8, u8, u64, u64)> = plan
            .ops
            .iter()
            .map(|op| match op {
                Op::AddNote { note, .. } => (
                    note.pitch,
                    note.velocity,
                    note.start_tick,
                    note.duration_ticks,
                ),
                other => panic!("必须是 AddNote: {other:?}"),
            })
            .collect();
        mapped.sort_unstable();
        assert_eq!(
            mapped,
            vec![(60, 100, 960, 960), (64, 64, 0, 480)],
            "tick 乘 2、音高与力度原样、时值取 end-start（同乘 2）"
        );
    }

    /// 判据 b（拒绝空导入）：文件里没有音符 ⇒ `Empty`，⛔ 不发出空提交。
    #[test]
    fn an_empty_file_is_refused_instead_of_committing_nothing() {
        let project = yeban_model::samples::filled_project();
        let error = plan_midi_import(
            &project,
            &parsed(960, Vec::new()),
            &MidiImportOptions::default(),
        )
        .expect_err("空文件必须被拒绝");
        assert!(
            matches!(error, MidiImportError::Empty(_)),
            "实际: {error:?}"
        );
    }

    /// 一段**最小合法 SMF**（格式 0、960 PPQ、一个 C4 音符）。
    ///
    /// 手写字节而不是借用导出器：本判据要独立于"导出"那条路（否则来源判据会随导出器一起漂移）。
    fn minimal_smf() -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes()); // 格式 0
        bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
        bytes.extend_from_slice(&960u16.to_be_bytes()); // 960 PPQ
        let mut track: Vec<u8> = Vec::new();
        track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]); // tempo 500000
        track.extend_from_slice(&[0x00, 0x90, 0x3C, 0x64]); // note on C4 vel 100
        track.extend_from_slice(&[0x87, 0x40, 0x80, 0x3C, 0x40]); // delta 960: note off
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&track);
        bytes
    }

    /// 判据（父单：**钉住来源标记**，⛔ 不靠代码写法保证）。
    ///
    /// ## 这条判据在什么条件下会红（**有牙**）
    ///
    /// 把 `import_midi_bytes` 里的
    /// `commit_ops_with_origin(now_ms, OpOrigin::MidiInput, …)` 换成 `commit_ops(now_ms, …)`
    /// ⇒ 端口记下的来源变成 `UserUi` ⇒ 第一条断言红。**已实测注入**（临时改一次、观察变红、再改回）。
    ///
    /// ## 为什么还带一个**负对照**
    ///
    /// 只断言 `== Some(MidiInput)` 不足以证明这个读数**有判别力**（万一 `last_commit_origin`
    /// 恒返回 `MidiInput` 呢）⇒ 同一条判据里再用**界面入口**提交一次，要求它记成 `UserUi`。
    #[test]
    fn the_import_commit_is_stamped_midi_input_and_the_ui_entry_still_reads_user_ui() {
        let project = yeban_model::samples::filled_project();
        let session =
            crate::undo::UndoSession::open("<判据:导入来源>", "yeban-app", project.clone(), 1)
                .expect("打开会话");
        let port = crate::undo::UndoPort::new(session);
        assert_eq!(port.last_commit_origin(), None, "还没提交过 ⇒ 无来源读数");

        let notes_before: usize = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        let report = import_midi_bytes(
            &port,
            &project,
            &minimal_smf(),
            &MidiImportOptions::default(),
            10,
        )
        .expect("最小 SMF 必须能导入");
        assert_eq!(report.notes, 1, "只导入了文件里的那一个音符");
        // ① 核心语义：这次提交的来源是 `MidiInput`。
        assert_eq!(
            port.last_commit_origin(),
            Some(OpOrigin::MidiInput),
            "导入提交必须被标成 `MidiInput`（⛔ 不是 `UserUi`）"
        );
        assert_eq!(report.origin, OpOrigin::MidiInput, "报告与端口读数必须一致");
        let notes_after: usize = port
            .project()
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .map(std::collections::BTreeMap::len)
            .sum();
        assert_eq!(
            notes_after,
            notes_before + report.notes,
            "这次提交必须真的带上全部导入 op"
        );

        // ② 负对照：界面入口仍记成 `UserUi` ⇒ 来源读数**有判别力**。
        // （只关心来源标记，所以这个 op 的坐标取固定值即可，⛔ 不需要走铅笔换算。）
        let (track_id, clip_id) =
            destination(&project, &MidiImportOptions::default()).expect("必须能找到目标");
        let op = Op::AddNote {
            track_id,
            clip_id,
            note: MidiNote::new(EntityId::new(), 0, 60, 240),
        };
        port.commit_ops(11, "ui: add note", vec![op])
            .expect("界面入口提交");
        assert_eq!(
            port.last_commit_origin(),
            Some(OpOrigin::UserUi),
            "界面入口必须记成 `UserUi`（否则上面那条断言没有判别力）"
        );
    }

    /// 判据 c（拒绝非法字节）：真的喂一段非 SMF 字节 ⇒ `Parse`，且**不产生任何操作**。
    #[test]
    fn invalid_bytes_are_refused_before_any_operation_is_built() {
        let project = yeban_model::samples::filled_project();
        let session =
            crate::undo::UndoSession::open("<判据:import-midi>", "yeban-app", project.clone(), 1)
                .expect("打开会话");
        let port = crate::undo::UndoPort::new(session);
        let error = import_midi_bytes(
            &port,
            &project,
            b"this is not a midi file",
            &MidiImportOptions::default(),
            2,
        )
        .expect_err("非法字节必须被拒绝");
        assert!(
            matches!(error, MidiImportError::Parse(_)),
            "实际: {error:?}"
        );
        assert_eq!(
            port.last_commit_origin(),
            None,
            "解析失败 ⇒ ⛔ 一次提交都不许发生"
        );
        assert_eq!(
            port.project().clip_pool.len(),
            project.clip_pool.len(),
            "工程必须逐字未变"
        );
    }
}
