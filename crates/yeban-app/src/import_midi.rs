//! MIDI 文件**导入**：把一段 SMF 里的音符变成工程里的**真实编辑操作**，并如实标注来源。
//!
//! ## 物理架构与纯映射下移 (`[ARCH-EXT-002]`, `#344`)
//!
//! SMF 纯映射（[`plan_midi_import`]、[`MidiImportPlan`]、[`MidiImportLosses`]、
//! [`destination`] 等）已下移至 [`yeban_midi::import`]。
//! 本模块通过 `pub use yeban_midi::import::*;` 重新导出全部纯映射契约，
//! 保持 100% 源码兼容性。
//!
//! 本模块仅承载与工程编辑会话、撤销栈（[`UndoPort`]）绑定的入口：
//! - [`import_midi_bytes`]
//! - [`import_midi_file`]
//! - [`MidiImportReport`]
//!
//! ## 数据流（每一步都复用既有设施，⛔ 无旁路）
//!
//! ```text
//! bytes ──▶ yeban_midi::midi::parse_smf（既有解析器）
//!       ──▶ plan_midi_import（yeban_midi::import 纯函数）
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

pub use yeban_midi::import::*;
use yeban_midi::midi::parse_smf;
use yeban_model::{EntityId, OpOrigin, YebanProjectV1};

use crate::undo::UndoPort;

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
    /// ⭐ `#317`：歌词的读取／归属／丢弃读数（⛔ 不沉默地少做）。
    pub lyrics: MidiLyricCounts,
    /// ⭐ `#318`：音符/歌词以外的逐类丢失读数（`format` / `file-tracks` /
    /// `track-partitions-dropped` / `channels` 是上下文，其余是 read/dropped 两列）。
    pub losses: MidiImportLosses,
}

impl MidiImportReport {
    /// 一行可读摘要（CLI 打印它；`origin` 用可检索的短名）。
    ///
    /// ⭐ `#317`：`lyrics= attached= dropped=` 是三类**互不重叠**的读数；
    /// `non-utf8=` / `empty=` 是另外两类（见 [`MidiLyricCounts`]）。恒等式
    /// `lyrics == attached + dropped` 由 [`MidiLyricCounts::is_consistent`] 自检。
    ///
    /// ⭐ `#318`：在歌词五读数之后追加——
    /// - **上下文（总是打印）**：`format=` / `file-tracks=` / `track-partitions-dropped=` /
    ///   `channels=`（值再小也不是"没有这个读数"）；
    /// - **每个 aux 事件类**（仅 `read>0` 才打印，避免普通文件变长）：
    ///   `<kind>-read=N <kind>-dropped=N`（键名见 [`yeban_midi::midi::MidiAuxEventKind::key`]）；
    /// - **两个按音符计的属性**：`note-channel-*` / `note-off-velocity-*`
    ///   （`read == dropped == notes`，今天）。
    ///
    /// ⚠ `dropped == read` 今天只对**无落点**的 14 类成立；有落点的十类（`#342`
    /// 起四类、后续 ADR 扩到十类）`dropped` 可能小于 `read`，且 `dropped > 0` 时
    /// 会多一段 `<kind>-reason=<精确原因>`。⛔ 不许删判据。
    #[must_use]
    pub fn summary(&self) -> String {
        let origin = match self.origin {
            OpOrigin::MidiInput => "midi-input",
            OpOrigin::UserUi => "user-ui",
            _ => "other",
        };
        // ⭐ `#319`：`format=` 起的整段尾部（上下文 ＋ 按音符 ＋ 24 类逐类）由
        // [`MidiImportLosses::loss_detail`] **唯一**格式化；GUI 的 `loss_detail="…"`
        // 字段读的是**同一串** ⇒ 两个出口不可能各说各话（CLI 行逐字不变）。
        format!(
            "imported-midi: path={} notes={} {} track={} clip={} commit={} origin={origin} {}",
            self.path
                .as_ref()
                .map_or_else(|| "<bytes>".to_owned(), |path| path.display().to_string()),
            self.notes,
            // ⭐ `#320`：五个歌词读数由 `MidiLyricCounts` 的 `Display` **唯一**格式化；
            // GUI 的 `lyrics_detail="…"` 打印同一串 ⇒ CLI 行逐字不变。
            self.lyrics,
            self.track_id.to_canonical_string(),
            self.clip_id.to_canonical_string(),
            self.commit.to_canonical_string(),
            self.losses.loss_detail(),
        )
    }
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
        lyrics: plan.lyrics,
        losses: plan.losses,
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
    use yeban_model::{MidiNote, Op};

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

#[cfg(test)]
mod metadata_adoption_tests {
    use super::*;
    use crate::undo::UndoPort;
    use yeban_midi::midi::MidiAuxEventKind;

    /// 造一份**只差 tempo** 的最小 SMF：1 条轨、960 PPQ、一个 C4 音符、⭐ 指定的
    /// `(tick, µs/四分音符)` 序列（tick 是全轨绝对 tick）。
    fn smf_with_tempos(tempos: &[(u32, u32)]) -> Vec<u8> {
        fn vlq(mut value: u32) -> Vec<u8> {
            let mut out = vec![u8::try_from(value & 0x7F).expect("7 bit")];
            value >>= 7;
            while value > 0 {
                out.push(0x80 | u8::try_from(value & 0x7F).expect("7 bit"));
                value >>= 7;
            }
            out.reverse();
            out
        }
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes()); // 格式 0
        bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
        bytes.extend_from_slice(&960u16.to_be_bytes()); // 960 PPQ
        let mut track: Vec<u8> = Vec::new();
        let mut cursor = 0u32;
        for &(tick, micros) in tempos {
            track.extend(vlq(tick - cursor));
            cursor = tick;
            // `FF 51 03 <3 字节大端 µs/四分音符>`
            track.extend_from_slice(&[0xFF, 0x51, 0x03]);
            track.extend_from_slice(&micros.to_be_bytes()[1..]);
        }
        // 音符在 tempo 之后（tick 仍是 0 或已推进的 cursor —— 用 delta 对齐到 960）。
        let note_tick = 960u32.max(cursor);
        track.extend(vlq(note_tick - cursor));
        track.extend_from_slice(&[0x90, 0x3C, 0x64]); // note on C4 vel 100
        track.extend_from_slice(&[0x87, 0x40, 0x80, 0x3C, 0x40]); // delta 960: note off
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&track);
        bytes
    }

    /// ⭐ `#342` **主判据（单值可以落）**：文件只有**唯一条 tick 0 tempo** ⇒ 它就是
    /// 整份文件的 tempo ⇒ 落进 `project.bpm`（`SetTempo`）。
    #[test]
    fn a_single_tick_zero_tempo_is_adopted_into_the_project_bpm() {
        // 250_000 µs/四分音符 = **240 BPM**（与工程侧 90 BPM 明显不同）。
        const EXTREME_MICROS: u32 = 250_000;
        let bytes = smf_with_tempos(&[(0, EXTREME_MICROS)]);

        // ⭐ 控制臂（前置）：文件**真的**携带 240 BPM，⛔ 不是"看起来极端"。
        let parsed_file = yeban_midi::midi::parse_smf(&bytes).expect("解析自造 SMF");
        assert_eq!(parsed_file.tempos.len(), 1, "文件必须真的带一条 tempo");
        assert_eq!(
            parsed_file.tempos[0].microseconds_per_quarter,
            Some(EXTREME_MICROS),
            "文件带的必须是那个极端 tempo"
        );
        assert_eq!(parsed_file.tempos[0].tick, 0, "唯一条 tempo 必须在 tick 0");

        // 工程侧：**没有内容**（否则内容门槛会拒绝采纳）+ `bpm = 90`。
        let mut project = content_free_project();
        project.bpm = 90.0;
        let session = crate::undo::UndoSession::open(
            "<判据:单值 tempo 落进 bpm>",
            "yeban-app",
            project.clone(),
            1,
        )
        .expect("打开会话");
        let port = UndoPort::new(session);

        let report = import_midi_bytes(&port, &project, &bytes, &MidiImportOptions::default(), 10)
            .expect("单 tempo 的 SMF 必须能导入");

        assert_eq!(
            report.notes, 1,
            "⭐ 控制臂：音符必须真的进来了（⛔ 否则'`bpm` 变了'可能只是没导入）"
        );
        assert_eq!(
            (
                report.losses.aux_events[&MidiAuxEventKind::Tempo].read,
                report.losses.aux_events[&MidiAuxEventKind::Tempo].dropped
            ),
            (1, 0),
            "唯一条 tick 0 tempo ⇒ 落地，不是损失"
        );
        // ⭐ 主断言：工程的 `bpm` = 文件里的 240。
        assert_eq!(
            port.project().bpm,
            240.0,
            "唯一条 tick 0 tempo 是整份文件的 tempo ⇒ 落进 `project.bpm`（单标量能承载）"
        );
        assert_eq!(
            port.project().transport.count_in_bars,
            project.transport.count_in_bars,
            "⭐ 相邻字段（同一 `TransportConfig`）必须一字不变 —— 证明改的不是别处"
        );
    }

    /// ⭐ `#342`：工程**已有内容**（`filled_project`）⇒ 内容门槛拒绝 tempo 采纳：
    /// `project.bpm` **一字不变**，整类 `dropped` 且原因**点名两个值**（⛔ 不许静默
    /// 挑一个）。⚠ `#349` 起 map 本身有落点（空工程会落进 `project.tempo_map`，
    /// 见 `yeban_midi::import` 的同名判据）；这里拒收的理由是**内容门槛**，⛔ 不是"没有家"。
    #[test]
    fn a_tempo_map_does_not_touch_the_project_bpm_and_names_every_value() {
        // 240 BPM @0，120 BPM @960（两个值都与工程的 90 不同）。
        let bytes = smf_with_tempos(&[(0, 250_000), (960, 500_000)]);
        let parsed_file = yeban_midi::midi::parse_smf(&bytes).expect("解析自造 SMF");
        assert_eq!(parsed_file.tempos.len(), 2, "控制臂：文件真的有两条 tempo");

        let mut project = yeban_model::samples::filled_project();
        project.bpm = 90.0;
        let session = crate::undo::UndoSession::open(
            "<判据:tempo map 不落>",
            "yeban-app",
            project.clone(),
            1,
        )
        .expect("打开会话");
        let port = UndoPort::new(session);
        let report = import_midi_bytes(&port, &project, &bytes, &MidiImportOptions::default(), 10)
            .expect("两段 tempo 的 SMF 必须能导入");

        assert_eq!(report.notes, 1, "控制臂：音符必须真的进来了");
        assert_eq!(
            port.project().bpm,
            90.0,
            "tempo map 没有家 ⇒ ⛔ 不许挑一个值写进单标量 `project.bpm`"
        );
        assert_eq!(
            (
                report.losses.aux_events[&MidiAuxEventKind::Tempo].read,
                report.losses.aux_events[&MidiAuxEventKind::Tempo].dropped
            ),
            (2, 2),
            "两条 tempo 都丢"
        );
        let reason = &report.losses.aux_reasons[&MidiAuxEventKind::Tempo];
        assert!(
            reason.contains("240.00 BPM @0") && reason.contains("120.00 BPM @960"),
            "损失原因必须**点名两个值**与它们的 tick: {reason}"
        );
    }
}
