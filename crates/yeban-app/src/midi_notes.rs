//! ⭐ `--midi-in` 采到的消息 ⇒ **音乐**：谱面（工程音符）＋ 声音（离线渲染）。
//!
//! ## 路径（⭐ 每一环都是**既有**机制，⛔ 没有新造）
//!
//! ```text
//! 采到的原始消息 ──pair_notes──▶ CapturedNote[]
//!                                  │
//!        ┌─────────────────────────┴──────────────────────────┐
//!        ▼ ① 谱面（本模块）                                    ▼ ② 声音（CLI 调用既有导出器）
//!   Op::AddNote + UndoPort 提交                        export_wav::export_wav(工程)
//!   （来源 = OpOrigin::MidiInput，一次提交）            ＝ SynthEngine::arm_from_snapshot
//!        │                                                  + render_track（排程驱动）
//!        ▼                                                  │
//!   工程里**真的**多出音符（判据读回）                        ▼
//!                                                     24-bit WAV（判据听非静音 + 比字节）
//! ```
//!
//! ⚠ ⭐ 声音那一半**必须**经工程：引擎今天唯一有消费者的音符入口是
//! `SynthEngine::render_track(track, schedule, …)`（**排程驱动**，`--export-wav` 走的就是它）。
//!
//! ## ⚠ 断点登记（⭐ 实测，⛔ 不是猜测）：`EngineEvent::NoteOn` / `NoteOff` **今天没有消费者**
//!
//! 任务书设想的第二条路径（"把同一批事件推进 `EngineEvent` 环 ⇒ 离线渲染"）**今天不通**，
//! 证据（本机实测，2026-10-12）：
//!
//! 1. `crates/yeban-engine/src/ring.rs` 里 `NoteOn` / `NoteOff` **只有定义与判据在造它们**
//!    （`grep -rn "EngineEvent::NoteOn" crates/`：除 `ring.rs` 的测试外只剩
//!    `crates/yeban-engine/tests/rt_zero_alloc.rs`）；
//! 2. `crates/yeban-engine/src/rt.rs` 的事件出队闭包只匹配 **`SetParam`** 与 **`Transport`**
//!    （`grep -n "if let EngineEvent" rt.rs` ⇒ 两处）⇒ 音符事件被 `events_applied` **计数**
//!    却**没有任何效果**；
//! 3. `SynthEngine` **没有** `note_on` / `note_off` 之类的公开触发入口（公开音符入口只有
//!    `render_track`），因此引擎侧**连被调用的方法都还不存在**；
//! 4. 实测：把 4 条事件（两条 NoteOn + 两条 NoteOff）推进环后离线渲染
//!    （`EngineRuntime::process_quantum`，48 kHz 立体声 2 秒）⇒ `events_sent=4/4`、
//!    **非零样本 = 0、峰值 = 0.000000**（全静音）。
//!
//! ⇒ ⭐ 那条路径需要**先给 `yeban-engine` 接线**（`SynthEngine` 的音符触发 API ＋ 出队闭包里
//! 的一个 `if let EngineEvent::NoteOn/NoteOff` 分支）—— 那是**别的工作线的面**，本切片
//! **不碰**（⛔ 也不留"看起来接好了"的空管线）。本切片的"声音有反应"因此走**工程排程路径**：
//! 音符先真的落进工程（①），再由既有导出器渲染（②）⇒ 两步都能判、也都不需要声卡。
//!
//! 判据 [`tests::the_engine_event_note_path_is_registered_as_unwired`] 把这条断点**钉在
//! 源码文本上**：引擎一旦接线，它会红 ⇒ 那时请更新本节的登记与 `--midi-in-render` 的路径。
//!
//! ## 时间口径
//!
//! 采集来的时间戳是**微秒**，按 `[MODEL-AST-001]` 的 960 PPQ 换算成 tick
//! （复用 [`crate::midi_in::ticks_from_gap_us`] 的**同一个**算式；`midi_in.rs` 有一条判据
//! 钉住它与 `yeban_midi_io::ticks_from_origin_us` 逐值相等）。
//!
//! ⛔ **精度不发明**：本模块只做"消息 ⇒ 音符"的映射，音符的时间就是 MIDI 自己的时间戳
//! 换算来的 tick；quantize / 吸附仍归模型层（本模块不碰）。

use std::collections::BTreeMap;

use yeban_model::{EntityId, MidiNote, Op, OpOrigin, YebanProjectV1};

use crate::import_midi::{MidiImportOptions, destination};
use crate::undo::UndoPort;

/// ⭐ 模型的每四分音符 tick 数（960 PPQ，`[MODEL-AST-001]`）—— 与 `import_midi` 同源。
pub const MODEL_PPQ: u64 = crate::import_midi::MODEL_PPQ;

/// ⭐ 一条**未闭合**音符（收了 NoteOn、没收到 NoteOff）的收尾时长（tick）。
///
/// ⚠ 起算点是**采集末尾**（最后一条消息的 tick），⛔ 不是那条音符自己的起点 ——
/// 它一直按着，尾巴买的是"采集结束时它还没松开"这件事。
///
/// 120 tick = 一个十六分音符（960 PPQ 下 4 分音符 = 960）⇒ 这是**可听的**最短收尾，
/// 而不是"丢进队列等下次再说"。条数会如实记进 [`PairingReport::unclosed`]（⛔ 不静默）。
pub const UNCLOSED_TAIL_TICKS: u64 = 120;

/// ⭐ 一条采到的音符（时间是 960 PPQ 的 **tick**，⛔ 不是微秒）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct CapturedNote {
    /// MIDI 音高（0..=127）。
    pub pitch: u8,
    /// MIDI 力度（0..=127）。
    pub velocity: u8,
    /// 起始 tick。
    pub start_tick: u64,
    /// 结束 tick（`> start_tick`，保证模型层 `duration_ticks >= 1`）。
    pub end_tick: u64,
}

impl CapturedNote {
    /// 时长（tick，至少 1）。
    ///
    /// ⚠ 不是 `const fn`：`u64::max` 目前还不是常量 trait 方法（`E0658`，实测踩到）。
    #[must_use]
    pub fn duration_ticks(&self) -> u64 {
        self.end_tick.saturating_sub(self.start_tick).max(1)
    }
}

/// ⭐ 配对的**全部读数**（⚠ 被忽略的消息也要**计数**，⛔ 不静默丢）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PairingReport {
    /// 成对成功的音符（按 `(start_tick, pitch, end_tick)` 升序）。
    pub notes: Vec<CapturedNote>,
    /// 收到 NoteOn、没收到 NoteOff 的条数（按 [`UNCLOSED_TAIL_TICKS`] 收尾）。
    pub unclosed: usize,
    /// 没有对应 NoteOn 的 NoteOff 条数（忽略，但计数）。
    pub orphan_offs: usize,
    /// **重触发**条数：同一音高在已按下时又来一条 NoteOn ⇒ 前一条在**新起点**收尾。
    pub restarted: usize,
    /// 非音符消息条数（CC / 弯音 / 触后 / 实时字节 …）——忽略，但计数。
    pub ignored: usize,
}

impl PairingReport {
    /// 一行可读摘要（CLI 打印它）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "midi-notes-pairing: notes={} unclosed={} orphan-offs={} restarted={} ignored={} ppq={MODEL_PPQ}",
            self.notes.len(),
            self.unclosed,
            self.orphan_offs,
            self.restarted,
            self.ignored,
        )
    }
}

/// ⭐ **原始消息 ⇒ 成对音符**（纯函数：零 I/O、零硬件、零时间依赖 ⇒ 无声卡也判）。
///
/// ## 消息判定（逐条写死，⛔ 不猜）
///
/// | 字节 | 判定 |
/// | :--- | :--- |
/// | `status & 0xF0 == 0x90` 且 `data2 > 0` | NoteOn（`pitch = data1 & 0x7F`，`velocity = data2`） |
/// | `status & 0xF0 == 0x90` 且 `data2 == 0` | NoteOff（**running status 的常见写法**） |
/// | `status & 0xF0 == 0x80` | NoteOff |
/// | 其余（`0xA0`/`0xB0`/`0xC0`/`0xD0`/`0xE0`/系统消息） | 忽略 **但计数**（[`PairingReport::ignored`]） |
///
/// ## 时间
///
/// `tick = crate::midi_in::ticks_from_gap_us(timestamp_us - origin_us)` —— 与
/// `yeban_midi_io::ticks_from_origin_us` **同一个算式**（`midi_in.rs` 有一条判据钉住两者相等）。
#[must_use]
pub fn pair_notes(
    messages: &[(u64, u8, u8, u8)],
    origin_us: u64,
    unclosed_tail_ticks: u64,
) -> PairingReport {
    let mut open: BTreeMap<u8, (u64, u8)> = BTreeMap::new();
    let mut report = PairingReport::default();
    let mut last_tick = 0_u64;

    for (timestamp_us, status, data1, data2) in messages {
        let tick = crate::midi_in::ticks_from_gap_us(timestamp_us.saturating_sub(origin_us));
        last_tick = last_tick.max(tick);
        let pitch = data1 & 0x7F;
        match status & 0xF0 {
            0x90 if *data2 > 0 => {
                if let Some((start_tick, velocity)) = open.insert(pitch, (tick, *data2)) {
                    // ⭐ 重触发：前一条在**新起点**收尾（时长至少 1 tick）。
                    report.restarted += 1;
                    report.notes.push(CapturedNote {
                        pitch,
                        velocity,
                        start_tick,
                        end_tick: tick.max(start_tick + 1),
                    });
                }
            }
            0x90 | 0x80 => match open.remove(&pitch) {
                Some((start_tick, velocity)) => report.notes.push(CapturedNote {
                    pitch,
                    velocity,
                    start_tick,
                    end_tick: tick.max(start_tick + 1),
                }),
                None => report.orphan_offs += 1,
            },
            _ => report.ignored += 1,
        }
    }

    // ⭐ 未闭合的音符按 `last_tick + tail` 收尾（⛔ 不丢弃、⛔ 不留 0 时长）。
    let tail = last_tick.saturating_add(unclosed_tail_ticks);
    report.unclosed = open.len();
    for (pitch, (start_tick, velocity)) in open {
        report.notes.push(CapturedNote {
            pitch,
            velocity,
            start_tick,
            end_tick: tail.max(start_tick + 1),
        });
    }
    report
        .notes
        .sort_unstable_by_key(|note| (note.start_tick, note.pitch, note.end_tick));
    report
}

/// ⭐ 一次"把音符写进工程"的读数（CLI 打印它；判据逐字段读）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteWriteReport {
    /// 写进去的音符条数。
    pub notes: usize,
    /// 目标音轨。
    pub track_id: EntityId,
    /// 目标片段（摆放）。
    pub clip_id: EntityId,
    /// 本次提交的身份（撤销图里的提交 id）。
    pub commit: EntityId,
    /// 其中**未闭合**（按尾巴收尾）的条数 —— 从配对报告原样带过来。
    pub unclosed: usize,
}

impl NoteWriteReport {
    /// 一行可读摘要（`origin` 是全仓统一的可检索短名）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "midi-notes: notes={} track={} clip={} commit={} unclosed={} origin={}",
            self.notes,
            self.track_id.to_canonical_string(),
            self.clip_id.to_canonical_string(),
            self.commit.to_canonical_string(),
            self.unclosed,
            origin_short_name(OpOrigin::MidiInput),
        )
    }
}

/// `OpOrigin` 的可检索短名（与 `import_midi.rs` 的口径一致）。
fn origin_short_name(origin: OpOrigin) -> &'static str {
    match origin {
        OpOrigin::MidiInput => "midi-input",
        OpOrigin::UserUi => "user-ui",
        _ => "other",
    }
}

/// ⭐ 把采到的音符写进工程：`Op::AddNote` × N ⇒ **一次**带来源的提交。
///
/// 目标选择与 `--import-midi` **共用**同一条规则（[`crate::import_midi`] 的 `destination`）：
/// `track = None` ⇒ 第一条非 master 音轨 ＋ 它的第一个摆放；`track = Some(_)` ⇒ 指定的那条。
///
/// ⛔ 本函数**不**自己造 `MidiNote` 的规则、⛔ 不绕过 `UndoPort` 直接改工程：
/// 编辑必须经过撤销图（否则"弹出来的音符"就不能撤销，也不能进提交史）。
///
/// # Errors
///
/// [`MidiNoteError::NoNotes`]（一条音符都没有）、[`MidiNoteError::Destination`]
/// （没有目标音轨或摆放）、[`MidiNoteError::Refused`]（撤销端口拒绝这次提交）。
pub fn write_notes(
    port: &UndoPort,
    project: &YebanProjectV1,
    pairing: &PairingReport,
    track: Option<EntityId>,
    now_ms: u64,
) -> Result<NoteWriteReport, MidiNoteError> {
    if pairing.notes.is_empty() {
        return Err(MidiNoteError::NoNotes);
    }
    let (track_id, clip_id) = destination(
        project,
        &MidiImportOptions {
            track_id: track,
            clip_id: None,
        },
    )
    .map_err(|error| MidiNoteError::Destination(error.to_string()))?;

    let mut ops: Vec<Op> = Vec::with_capacity(pairing.notes.len());
    for note in &pairing.notes {
        let mut model_note = MidiNote::new(
            EntityId::new(),
            note.start_tick,
            note.pitch,
            note.duration_ticks(),
        );
        model_note.velocity = note.velocity;
        ops.push(Op::AddNote {
            track_id,
            clip_id,
            note: model_note,
        });
    }

    let commit = port
        .commit_ops_with_origin(now_ms, OpOrigin::MidiInput, "midi input notes", ops)
        .map_err(|refusal| MidiNoteError::Refused(refusal.to_string()))?;

    Ok(NoteWriteReport {
        notes: pairing.notes.len(),
        track_id,
        clip_id,
        commit,
        unclosed: pairing.unclosed,
    })
}

/// ⭐ 缺省目标音轨：**第一条非 master 音轨**（`BTreeMap` 键序 = 身份序 ⇒ 跨进程确定性）。
///
/// 与 `--import-midi` 的缺省目标**同一条规则**（复用 `import_midi::destination` 的判定）。
///
/// # Errors
///
/// [`MidiNoteError::Destination`]：工程里没有非 master 音轨。
pub fn default_track(project: &YebanProjectV1) -> Result<EntityId, MidiNoteError> {
    project
        .tracks
        .keys()
        .copied()
        .find(|id| *id != project.master_bus_track_id)
        .ok_or_else(|| MidiNoteError::Destination("工程里没有非 master 的音轨".to_owned()))
}

/// ⭐ `--midi-in-track` 的选择器 ⇒ 音轨身份。
///
/// ## 规则（与 `--midi-in` 的**端口**选择同一套 ⇒ 用户只学一次）
///
/// 1. 取值**全是十进制数字** ⇒ 按**序号**（非 master 音轨的身份序，`0` 起）；
/// 2. 否则按音轨**名字子串**（⭐ **大小写敏感**，与端口规则一致）；
/// 3. 命中 **0** 个 ⇒ [`MidiNoteError::NoSuchTrack`]（**列出可用音轨**）；
/// 4. 命中 **>1** 个 ⇒ [`MidiNoteError::AmbiguousTrack`]（**列出候选**、⛔ 不取第一条）。
///
/// # Errors
///
/// 见上 3 / 4。
pub fn resolve_track(project: &YebanProjectV1, selector: &str) -> Result<EntityId, MidiNoteError> {
    let ids: Vec<EntityId> = project
        .tracks
        .keys()
        .copied()
        .filter(|id| *id != project.master_bus_track_id)
        .collect();
    let names: Vec<String> = ids
        .iter()
        .filter_map(|id| project.tracks.get(id))
        .map(|track| track.name.clone())
        .collect();
    let trimmed = selector.trim();

    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return trimmed
            .parse::<usize>()
            .ok()
            .and_then(|index| ids.get(index).copied())
            .ok_or_else(|| MidiNoteError::NoSuchTrack {
                wanted: selector.to_owned(),
                available: names.clone(),
            });
    }

    let matches: Vec<EntityId> = ids
        .iter()
        .copied()
        .filter(|id| {
            project
                .tracks
                .get(id)
                .is_some_and(|track| track.name.contains(trimmed))
        })
        .collect();
    match matches.as_slice() {
        [] => Err(MidiNoteError::NoSuchTrack {
            wanted: selector.to_owned(),
            available: names,
        }),
        [only] => Ok(*only),
        many => Err(MidiNoteError::AmbiguousTrack {
            wanted: selector.to_owned(),
            matches: many
                .iter()
                .filter_map(|id| project.tracks.get(id))
                .map(|track| track.name.clone())
                .collect(),
        }),
    }
}

/// 可用音轨清单的**人可读**渲染（空清单也要**明说**，⛔ 不留空白）。
fn render_track_list(available: &[String]) -> String {
    if available.is_empty() {
        "工程里没有任何可用音轨".to_owned()
    } else {
        format!("可用音轨: {}", available.join(" / "))
    }
}

/// ⭐ 本模块的失败面（每一种都带**精确**原因，⛔ 不 panic）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiNoteError {
    /// 一条音符都没有（采到 0 条 / 全是非音符消息）。
    NoNotes,
    /// 没有目标（没有非 master 音轨 / 目标轨没有任何摆放）——原样携带 `import_midi` 的裁决文本。
    Destination(String),
    /// `--midi-in-track` 的选择器命中 0 条音轨（**列出可用音轨**）。
    NoSuchTrack {
        /// 用户给的取值。
        wanted: String,
        /// 可用的音轨名（身份序）。
        available: Vec<String>,
    },
    /// `--midi-in-track` 的选择器命中多于一条（**列出候选**，⛔ 不取第一条）。
    AmbiguousTrack {
        /// 用户给的取值。
        wanted: String,
        /// 命中的候选。
        matches: Vec<String>,
    },
    /// 撤销端口拒绝了这次提交。
    Refused(String),
}

impl MidiNoteError {
    /// 这一档失败的退出码（与 `--midi-in` / `--import-midi` 同一桶 = [`crate::cli::EXIT_OPEN`]）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        crate::cli::EXIT_OPEN
    }
}

impl core::fmt::Display for MidiNoteError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::NoNotes => write!(
                formatter,
                "采到的消息里一条音符都没有（只有非音符消息，或一个字节都没收到）"
            ),
            Self::Destination(detail) => write!(formatter, "没有可写的目标: {detail}"),
            Self::NoSuchTrack { wanted, available } => write!(
                formatter,
                "音轨选择器 `{wanted}` 命中 0 条音轨; {}",
                render_track_list(available)
            ),
            Self::AmbiguousTrack { wanted, matches } => write!(
                formatter,
                "音轨选择器 `{wanted}` 命中 {} 条音轨, ⛔ 不猜哪一条: {}",
                matches.len(),
                matches.join(" / ")
            ),
            Self::Refused(detail) => write!(formatter, "提交被撤销端口拒绝: {detail}"),
        }
    }
}

impl core::error::Error for MidiNoteError {}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一段**可判**的注入消息：C4 NoteOn → C4 NoteOff → E4 NoteOn（不闭合）＋ 一条 CC7。
    fn injected() -> Vec<(u64, u8, u8, u8)> {
        vec![
            (0, 0x90, 60, 100),
            (500_000, 0x80, 60, 0),
            (1_000_000, 0x90, 64, 90),
            (1_200_000, 0xB0, 7, 100), // CC7（非音符 ⇒ 被计数忽略）
        ]
    }

    /// 判据 ①（谱面）：⭐ **注入 N 条消息 ⇒ 成对出 N 条音符**，且音高 / 力度 / 起止 tick 逐项对得上。
    ///
    /// `0.5 s` 在 120 BPM 下 = 480 tick（960 PPQ）；未闭合的那条按**采集末尾 + 尾巴**收尾。
    #[test]
    fn pairing_turns_injected_messages_into_notes_with_exact_ticks() {
        let pairing = pair_notes(&injected(), 0, UNCLOSED_TAIL_TICKS);
        assert_eq!(pairing.notes.len(), 2, "两条音符: {}", pairing.summary());
        assert_eq!(pairing.ignored, 1, "CC7 必须被计数忽略");
        assert_eq!(pairing.unclosed, 1, "E4 没有 NoteOff");
        assert_eq!(pairing.orphan_offs, 0);
        assert_eq!(pairing.restarted, 0);

        let c4 = pairing.notes[0];
        assert_eq!((c4.pitch, c4.velocity), (60, 100));
        assert_eq!(c4.start_tick, 0);
        let half_second = crate::midi_in::ticks_from_gap_us(500_000);
        assert_eq!(c4.end_tick, half_second, "0.5 s 必须换算成 480 tick");
        assert_eq!(half_second, 480);

        let e4 = pairing.notes[1];
        assert_eq!((e4.pitch, e4.velocity), (64, 90));
        assert_eq!(e4.start_tick, crate::midi_in::ticks_from_gap_us(1_000_000));
        // ⭐ 未闭合的音符在**采集末尾**（最后一条消息的 tick）＋ 尾巴处收尾 —— ⛔ 不是
        // 从它自己的起点算尾巴：它一直按着，尾巴是"采集结束时它还没松开"的代价。
        let last_tick = crate::midi_in::ticks_from_gap_us(1_200_000);
        assert_eq!(
            e4.end_tick,
            last_tick + UNCLOSED_TAIL_TICKS,
            "未闭合的按采集末尾 + 尾巴收尾"
        );
        assert!(e4.duration_ticks() >= 1);
    }

    /// 判据 ②（谱面）：⭐ `NoteOn(vel=0)` 必须被当作 NoteOff（running status 的常见写法），
    /// 且**重触发**会让前一条在新起点收尾（如实计数，⛔ 不丢）。
    #[test]
    fn a_zero_velocity_note_on_closes_and_a_retrigger_is_counted() {
        let messages = vec![
            (0, 0x90, 60, 100),
            (100_000, 0x90, 60, 0),  // vel=0 ⇒ NoteOff
            (200_000, 0x90, 60, 80), // 再按
            (300_000, 0x90, 60, 70), // 重触发（没先松开）
            (400_000, 0x80, 60, 0),  // 松开
            (500_000, 0x80, 72, 0),  // 孤儿 NoteOff
        ];
        let pairing = pair_notes(&messages, 0, UNCLOSED_TAIL_TICKS);
        assert_eq!(pairing.notes.len(), 3, "{}", pairing.summary());
        assert_eq!(pairing.restarted, 1, "重触发必须计数");
        assert_eq!(pairing.orphan_offs, 1, "孤儿 NoteOff 必须计数");
        assert_eq!(pairing.unclosed, 0);
        assert_eq!(
            pairing.notes[0].end_tick,
            crate::midi_in::ticks_from_gap_us(100_000)
        );
        assert_eq!(
            pairing.notes[1].end_tick,
            crate::midi_in::ticks_from_gap_us(300_000)
        );
        assert_eq!(
            pairing.notes[2].end_tick,
            crate::midi_in::ticks_from_gap_us(400_000)
        );
    }

    /// 判据 ③（目标选择）：⭐ 序号 / 名字子串 / 0 命中 / >1 命中 —— 与 `--midi-in` 的
    /// **端口**规则同一套；⭐ 主总线**永远不可选**（它不是一个能收音符的目标）。
    #[test]
    fn the_track_selector_follows_the_same_rules_as_the_port_selector() {
        let project = crate::bridge::demo_project();
        let ids: Vec<EntityId> = project
            .tracks
            .keys()
            .copied()
            .filter(|id| *id != project.master_bus_track_id)
            .collect();
        assert_eq!(
            ids.len(),
            6,
            "演示夹具的非 master 音轨数变了 ⇒ 本判据需同步"
        );

        // ⭐ 缺省 = 第一条非 master（身份序）。
        assert_eq!(default_track(&project).expect("有音轨"), ids[0]);

        // ⭐ 全十进制 ⇒ 序号。
        assert_eq!(resolve_track(&project, "0").expect("序号 0"), ids[0]);
        assert_eq!(resolve_track(&project, " 5 ").expect("序号 5"), ids[5]);
        assert!(
            matches!(
                resolve_track(&project, "6"),
                Err(MidiNoteError::NoSuchTrack { .. })
            ),
            "越界的序号必须被拒"
        );
        // ⭐ 主总线不在候选里 ⇒ 它的名字选中不了。
        let master_name = &project
            .tracks
            .get(&project.master_bus_track_id)
            .expect("有主总线")
            .name;
        assert!(
            matches!(
                resolve_track(&project, master_name),
                Err(MidiNoteError::NoSuchTrack { .. })
            ),
            "主总线不可作为音符目标: {master_name}"
        );

        // ⭐ 名字子串（对中文名字同样成立：子串必须真的在里面）。
        let first_name = project.tracks.get(&ids[0]).expect("在").name.clone();
        assert_eq!(
            resolve_track(&project, &first_name).expect("唯一命中"),
            ids[0]
        );
        let error = resolve_track(&project, "没有这条轨").expect_err("0 命中必须报错");
        match &error {
            MidiNoteError::NoSuchTrack { available, .. } => {
                assert_eq!(available.len(), 6, "必须列出全部可用音轨");
                assert!(
                    available.contains(&first_name),
                    "清单里必须有它: {available:?}"
                );
            }
            other => panic!("必须是 NoSuchTrack，得到 {other:?}"),
        }
        // ⭐ 空串匹配**所有**音轨 ⇒ 歧义（⛔ 不默默取第一条）。
        assert!(
            matches!(
                resolve_track(&project, ""),
                Err(MidiNoteError::AmbiguousTrack { .. })
            ),
            "空选择器必须报歧义"
        );
        // ⭐ 没有音轨的工程：清单为空时**明说**，⛔ 不留空白。
        let empty = YebanProjectV1::default();
        let error = default_track(&empty).expect_err("没有音轨必须报错");
        assert!(error.to_string().contains("没有非 master"), "{error}");
        let error = resolve_track(&empty, "鼓").expect_err("没有音轨必须报错");
        assert!(error.to_string().contains("没有任何可用音轨"), "{error}");
        assert_eq!(error.exit_code(), crate::cli::EXIT_OPEN);
    }

    /// 判据 ④（**断点登记**）：⭐ 任务书设想的 `EngineEvent` 音符路径**今天没有消费者** ——
    /// 这条判据把那个事实**钉在源码文本上**，因此它不会随着口头约定漂走。
    ///
    /// ⚠ 本判据**红了意味着引擎线接线了**（`rt.rs` 的出队闭包里多了 `NoteOn`/`NoteOff` 分支，
    /// 或 `SynthEngine` 多了音符触发入口）⇒ 那时应当把本模块头部的"断点登记"改成"已接线"，
    /// 并重新评估 `--midi-in-render` 走**事件环路径**是否比工程排程路径更合适。
    /// ⛔ 本切片不替引擎线做那件事（那是别的工作线的面）。
    #[test]
    fn the_engine_event_note_path_is_registered_as_unwired() {
        let ring = include_str!("../../yeban-engine/src/ring.rs");
        let runtime = include_str!("../../yeban-engine/src/rt.rs");
        let synth = include_str!("../../yeban-engine/src/synth.rs");

        // 定义还在（⛔ 不是"变体被删了"）。
        assert!(ring.contains("NoteOn {"), "EngineEvent::NoteOn 必须仍在");
        assert!(ring.contains("NoteOff {"), "EngineEvent::NoteOff 必须仍在");
        // 出队闭包里**没有**音符分支 ⇒ 事件被计数但无声。
        let drain = runtime
            .split("events.drain_with(")
            .nth(1)
            .and_then(|rest| rest.split("});").next())
            .expect("必须能找到事件出队闭包");
        assert!(
            !drain.contains("NoteOn") && !drain.contains("NoteOff"),
            "引擎接线了音符事件 ⇒ 请更新本判据与 `midi_notes.rs` 的断点登记"
        );
        // 合成器没有音符触发入口（只有排程驱动的 `render_track`）。
        assert!(
            !synth.contains("pub fn note_on") && !synth.contains("pub fn note_off"),
            "SynthEngine 接线了音符触发 ⇒ 请更新本判据与断点登记"
        );
    }
}
