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
//! ## ⭐ 声音那一半走**环**（引擎已接线，`eca718ab`）
//!
//! 采集来的事件推 [`yeban_engine::ring::EngineEvent::NoteOn`] / `NoteOff` 进**既有**的
//! UI→引擎 SPSC（[`crate::engine_host::EngineHost::send_engine_events`]），再离线渲染
//! （[`render_engine_offline`] → [`crate::engine_host::EngineHost::render_into`]，
//! 与设备回调**同一个** `EngineRuntime::process_quantum`）⇒ 产物是**事件真的作用到合成器**
//! 的声音（判据用 [`yeban_engine::rt::EngineStats::poly_notes_triggered`] 作见证）。
//!
//! ⚠ **历史（这条登记的价值就在这里）**：`eca718ab` 之前，`EngineEvent::NoteOn` / `NoteOff`
//! 在引擎里**没有消费者**（出队闭包只认 `SetParam` / `Transport`；`SynthEngine` 连
//! `note_on`/`note_off` 都没有）⇒ 当时把事件推进环**只会得到全静音**（实测 4/4 事件进环、
//! 非零样本 0）。那时本模块的登记判据把这件事**钉在源码文本上**，并在引擎接线的那一刻
//! **变红提醒**（实测：`eca718ab` 落地后该判据 panic 在 `src/midi_notes.rs:625`）。
//! 现在它翻成"**已接线**"的形态（[`tests::the_engine_event_note_path_is_wired_to_the_synth`]），
//! 继续钉住**当前事实**：引擎确有那两条分支 ＋ app 侧**走的是它**。
//!
//! ### ⚠ 离线渲染必须先**预热一个量子**（本机实测的引擎行为，已登记）
//!
//! `EngineRuntime::render_block` 的步骤顺序是「**① 出队并应用事件 → ② `snapshot.begin_block()`
//! 武装快照/槽位**」⇒ 在**第一个**块里事件到达时槽位**还没武装** ⇒ `SynthEngine::note_on`
//! 早退返回 `false` ⇒ ⭐ **tick 0 的环音符会被丢掉**（实测：`C4@tick0` 单独渲染 ⇒
//! `poly_notes_triggered=0`、`nonzero=0`、它的 `NoteOff` 被记成 `orphan_note_offs=1`；
//! 同一条音符放在 tick 960 就正常发声）。
//!
//! [`render_engine_offline`] 因此**先推一个空量子**（输出丢弃）把快照武装好再开始记账
//! ⇒ 加了之后 `C4@tick0` 与 `E4@tick960` 两条都发声（`poly_notes_triggered=2`、
//! `nonzero_samples=62032`）。
//!
//! ⚠ 这是**绕行**：缺陷本体在引擎的步骤顺序（真机现场不受影响 —— 用户弹第一个音之前引擎
//! 早已跑过许多块），受影响的是"离线渲染从第 0 帧就推事件"这条用法。已在回报里作为
//! **跨线发现**登记给引擎线，⛔ 本切片不改 `yeban-engine`。
//!
//! ## 两条路并存（开关区分，⛔ 不是二选一）
//!
//! | 路 | 开关 | 产物 / 性质 |
//! | :--- | :--- | :--- |
//! | **谱面** | `--midi-in-write-notes` | 音符经 `Op::AddNote` 进工程（可撤销、进提交史、可 `--save-as` / 导出） |
//! | **声音** | `--midi-in-render <path>` | 同一批事件 ⇒ `EngineEvent` 环 ⇒ 离线渲染 24-bit WAV（**不需要**先写工程） |
//!
//! ⭐ 声音**不必**经工程：音符落不落进模型是**产品的另一件事**（可编辑性），而"弹一下有没有
//! 声音"由环负责。两者可以同给（报告行分别说明各自的结果）。
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

use yeban_engine::ring::EngineEvent;
use yeban_model::{EntityId, MidiNote, Op, OpOrigin, YebanProjectV1};

use crate::engine_host::EngineHost;
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

/// ⭐ 一个 tick 等于多少帧（整数算式；`bpm_millis = 0` 时按 120 BPM 兜底）。
///
/// 口诀：`帧/tick = 采样率 × 60_000 / (bpm_millis × 960)` —— 48000 Hz / 120 BPM ⇒ **25 帧/tick**。
#[must_use]
pub fn frames_per_tick(sample_rate: u32, bpm_millis: u32) -> u64 {
    let bpm_millis = u64::from(if bpm_millis == 0 { 120_000 } else { bpm_millis });
    u64::from(sample_rate)
        .saturating_mul(60_000)
        .checked_div(bpm_millis.saturating_mul(MODEL_PPQ))
        .unwrap_or(0)
}

/// tick ⇒ 帧（整数乘法 + 整数除法 ⇒ 确定性，⛔ 不引入浮点漂移）。
#[must_use]
pub fn ticks_to_frames(tick: u64, sample_rate: u32, bpm_millis: u32) -> u64 {
    tick.saturating_mul(frames_per_tick(sample_rate, bpm_millis))
}

/// 帧 ⇒ tick（[`ticks_to_frames`] 的逆式；整数算式）。
#[must_use]
pub fn frames_to_ticks(frame: u64, sample_rate: u32, bpm_millis: u32) -> u64 {
    let bpm_millis = u64::from(if bpm_millis == 0 { 120_000 } else { bpm_millis });
    let denominator = u64::from(sample_rate).saturating_mul(60_000);
    frame
        .saturating_mul(bpm_millis.saturating_mul(MODEL_PPQ))
        .checked_div(denominator)
        .unwrap_or(0)
}

/// ⭐ 把速度折算成**千分之一 BPM 的整数**（与 `bridge` 的 `bpm_millis` 同口径）。
///
/// 为什么不用浮点算帧：`[ARCH-DET-001]` 要求"同一输入 ⇒ 同一产物"，而浮点在两架构上可能
/// 差 1 ULP（`ADR-0001 D32`）⇒ tick↔帧一律**整数**算。
#[must_use]
pub fn bpm_millis(bpm: f64) -> u32 {
    let millis = (bpm * 1000.0).round();
    if millis.is_finite() && millis > 0.0 && millis <= f64::from(u32::MAX) {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            millis as u32
        }
    } else {
        120_000
    }
}

/// ⭐ 音符 ⇒ **引擎事件调度表**（按 tick 升序；⛔ `EngineEvent` 本身不带时间戳）。
///
/// 同一 tick 上 **NoteOff 先于 NoteOn**（`kind` 0 < 1）：重触发时先关旧声部再开新的，
/// ⛔ 不会让两个声部在同一 tick 上打架。
#[must_use]
pub fn engine_schedule(notes: &[CapturedNote], track: EntityId) -> Vec<(u64, EngineEvent)> {
    let mut schedule: Vec<(u64, u8, EngineEvent)> = Vec::with_capacity(notes.len() * 2);
    for note in notes {
        schedule.push((
            note.start_tick,
            1,
            EngineEvent::NoteOn {
                track,
                pitch: note.pitch,
                velocity: note.velocity,
            },
        ));
        schedule.push((
            note.end_tick.max(note.start_tick + 1),
            0,
            EngineEvent::NoteOff {
                track,
                pitch: note.pitch,
            },
        ));
    }
    schedule.sort_by_key(|(tick, kind, _)| (*tick, *kind));
    schedule
        .into_iter()
        .map(|(tick, _kind, event)| (tick, event))
        .collect()
}

/// ⭐ 离线渲染的读数（每个数字都来自**真的**跑过的那一次）。
///
/// ⚠ 不是 `Eq`：`peak: f32`（浮点没有全序，⛔ 不假装有）。
#[derive(Debug, Clone, PartialEq)]
pub struct OfflineRenderReport {
    /// 渲染的帧数。
    pub frames: u64,
    /// 声道数。
    pub channels: u16,
    /// 采样率（Hz）。
    pub sample_rate: u32,
    /// **真正写进环**的事件条数（环满时会少）。
    pub events_sent: usize,
    /// 调度表里的**全部**事件条数（与 `events_sent` 不等 ⇒ 有事件没进环）。
    pub events_scheduled: usize,
    /// 非零样本个数（交错样本计 **个**，⛔ 不是帧、⛔ 不是字节）。
    pub nonzero_samples: usize,
    /// 峰值绝对值（`0.0` = 全静音）。
    pub peak: f32,
    /// ⭐ 本次渲染里**现场的**音符条数（＝这次弹的音符；⛔ 不含工程自己排程里的）。
    ///
    /// ⚠ 它是"谱面"与"声音"两条路**在报告里可区分**的一半：⭐ `midi-notes:` 行说的是
    /// "**写进工程**了几条"（可撤销 / 进提交史），本字段说的是"**这次声音里有几条现场音符**"
    /// —— 两者可以一有一无（只渲染不写谱面 / 只写谱面不渲染）⇒ ⛔ 报告必须说清。
    pub notes: usize,
    /// **合成器器件侧**累计触发的音符数（`EngineStats::poly_notes_triggered` 的**增量**）。
    ///
    /// ⚠ 它是"事件**真的**到了合成器"的见证：⛔ 不拿它单独举证发声（发声由
    /// `nonzero_samples`/`peak` 举证），但**两者一起**才排除"有声音但不是这些事件造成的"。
    pub poly_notes_triggered: u64,
    /// 落盘字节数（`None` = 只渲染没落盘）。
    pub bytes: Option<u64>,
}

impl OfflineRenderReport {
    /// 一行可读摘要（CLI 打印它；`source=` 明说声音来自哪条路径）。
    #[must_use]
    pub fn summary(&self, path: Option<&std::path::Path>) -> String {
        let target = path.map_or_else(|| "<内存>".to_owned(), |path| path.display().to_string());
        let silence = if self.nonzero_samples == 0 {
            " ⚠ 全零: 引擎没有发出任何声音 (目标音轨可能没有可发声的乐器 / 被静音)"
        } else {
            ""
        };
        format!(
            "midi-render: path={target} live-notes={} frames={} channels={} sample_rate={} \
             events={}/{} poly-notes-triggered={} nonzero-samples={} peak={:.6} bytes={} \
             source=engine(EngineEvent 环 + process_quantum){silence}",
            self.notes,
            self.frames,
            self.channels,
            self.sample_rate,
            self.events_sent,
            self.events_scheduled,
            self.poly_notes_triggered,
            self.nonzero_samples,
            self.peak,
            self.bytes
                .map_or_else(|| "<未落盘>".to_owned(), |bytes| bytes.to_string()),
        )
    }
}

/// ⭐ **离线渲染**调度表：把事件按量子边界推进引擎环，并把真实输出取回（交错 `f32`）。
///
/// ⚠ 定时精度 = **一个量子**（`DEFAULT_BLOCK_FRAMES` = 128 帧 ≈ 2.7 ms @48 kHz）：
/// 第 `q` 个量子渲染**之前**，把 tick 落在该量子**末尾之前**的事件整批写进环
/// ⇒ 事件在块边界生效。`EngineEvent` 本来就不带时间戳，这是引擎事件模型的既有边界。
///
/// # Errors
///
/// [`MidiNoteError::Empty`]（`frames == 0`）、[`MidiNoteError::NoNotes`]（调度表为空 ⇒
/// 必然静音 ⇒ 如实拒绝，⛔ 不写一个空 WAV 冒充成功）。
pub fn render_engine_offline(
    host: &mut EngineHost,
    schedule: &[(u64, EngineEvent)],
    frames: u64,
    channels: u16,
    sample_rate: u32,
    bpm_millis: u32,
) -> Result<(Vec<f32>, OfflineRenderReport), MidiNoteError> {
    if frames == 0 {
        return Err(MidiNoteError::Empty("请求的帧数为 0".to_owned()));
    }
    if schedule.is_empty() {
        return Err(MidiNoteError::NoNotes);
    }
    let channels = channels.max(1);
    let block = EngineHost::quantum_frames();
    let quantum_frames = u64::try_from(block).unwrap_or(128);
    let quanta = frames.div_ceil(quantum_frames);
    let capacity = usize::try_from(
        quanta
            .saturating_mul(quantum_frames)
            .saturating_mul(u64::from(channels)),
    )
    .unwrap_or(0);
    let mut samples = vec![0.0_f32; capacity];

    // ⭐ **预热一个量子**（本机实测的必要步骤，⛔ 不是保险）：`EngineRuntime::render_block`
    // 的步骤顺序是「**① 出队并应用事件 → ② `snapshot.begin_block()` 武装快照/槽位」⇒
    // 在**第一个**块里，事件到达时槽位**还没武装** ⇒ `SynthEngine::note_on` 早退返回 false
    // ⇒ ⭐ **tick 0 的环音符会被丢掉**（实测：C4@tick0 不发声、它的 NoteOff 被记成
    // `orphan_note_offs`；E4@tick960 正常发声）。因为本函数是**离线**渲染 ⇒
    // 先推一个**空量子**（输出丢弃）把快照武装好，再开始记账 ⇒ tick 0 的音符也能发声。
    //
    // ⚠ 这是**绕行**，缺陷本体在引擎的步骤顺序（已作为发现登记在回报里）：
    // 真机（设备回调）里引擎在用户弹第一个音之前早就跑过许多块，因此现场不受影响；
    // 受影响的是"离线渲染从第 0 帧开始就推事件"这条用法。
    if !schedule.is_empty() {
        let mut warmup = vec![0.0_f32; block * usize::from(channels)];
        host.render_into(&mut warmup, channels, 1);
    }

    // ⭐ 见证的**基线**：读数取"推事件之前"的器件计数 ⇒ 报告的增量只归因于**本次推的事件**。
    let before = host
        .engine_stats()
        .map_or(0, |stats| stats.poly_notes_triggered);

    let mut cursor = 0_usize;
    let mut events_sent = 0_usize;
    for index in 0..quanta {
        let block_end_frame = (index + 1).saturating_mul(quantum_frames);
        let block_end_tick = frames_to_ticks(block_end_frame, sample_rate, bpm_millis);
        let mut batch: Vec<EngineEvent> = Vec::new();
        while cursor < schedule.len() && schedule[cursor].0 <= block_end_tick {
            batch.push(schedule[cursor].1);
            cursor += 1;
        }
        events_sent = events_sent.saturating_add(host.send_engine_events(&batch));
        let start = usize::try_from(
            index
                .saturating_mul(quantum_frames)
                .saturating_mul(u64::from(channels)),
        )
        .unwrap_or(0);
        let end = start + block * usize::from(channels);
        if end <= samples.len() {
            host.render_into(&mut samples[start..end], channels, 1);
        }
    }

    let after = host
        .engine_stats()
        .map_or(before, |stats| stats.poly_notes_triggered);

    let keep = usize::try_from(frames.saturating_mul(u64::from(channels)))
        .unwrap_or(samples.len())
        .min(samples.len());
    samples.truncate(keep);

    let mut nonzero_samples = 0_usize;
    let mut peak = 0.0_f32;
    for sample in &samples {
        if *sample != 0.0 {
            nonzero_samples += 1;
        }
        peak = peak.max(sample.abs());
    }

    Ok((
        samples,
        OfflineRenderReport {
            notes: 0,
            frames,
            channels,
            sample_rate,
            events_sent,
            events_scheduled: schedule.len(),
            nonzero_samples,
            peak,
            poly_notes_triggered: after.saturating_sub(before),
            bytes: None,
        },
    ))
}

/// ⭐ **离线渲染并落盘**：渲染 ⇒ 量化（与 `--export-wav` **同一条**）⇒ 一次写成 WAV。
///
/// 量化与落盘走 [`crate::export_wav::write_interleaved_wav`]（`--export-wav` 的唯一实现），
/// 因此"同一个渲染结果"在两个导出器里是**同一串字节**。
///
/// # Errors
///
/// 见 [`render_engine_offline`]；外加 [`MidiNoteError::Wav`]（量化/落盘被拒，⛔ 不留半成品）。
#[allow(clippy::too_many_arguments)]
pub fn render_to_wav(
    host: &mut EngineHost,
    schedule: &[(u64, EngineEvent)],
    path: &std::path::Path,
    frames: u64,
    channels: u16,
    sample_rate: u32,
    bpm_millis: u32,
    seed: u64,
) -> Result<OfflineRenderReport, MidiNoteError> {
    let (samples, mut report) =
        render_engine_offline(host, schedule, frames, channels, sample_rate, bpm_millis)?;
    let (bytes, _bits) = crate::export_wav::write_interleaved_wav(
        path,
        &samples,
        channels.max(1),
        sample_rate,
        seed,
    )
    .map_err(MidiNoteError::Wav)?;
    report.bytes = Some(bytes);
    Ok(report)
}

/// ⭐ **音符 ⇒ 环 ⇒ WAV**（"声音"那一半的一次做完的入口；CLI 的 `--midi-in-render` 走它）。
///
/// ⭐ 为什么要有 notes 级的入口（而不是让调用方自己拼 `engine_schedule` + `render_to_wav`）：
/// 报告必须能说清"**这次声音里有几条现场音符**"（[`OfflineRenderReport::notes`]）——
/// 那是"谱面"与"声音"两条路在输出里**可区分**的一半（两者可以一有一无）。
///
/// # Errors
///
/// 见 [`render_engine_offline`] 与 [`render_to_wav`]。
#[allow(clippy::too_many_arguments)]
pub fn render_notes_to_wav(
    host: &mut EngineHost,
    notes: &[CapturedNote],
    track: EntityId,
    path: &std::path::Path,
    frames: u64,
    channels: u16,
    sample_rate: u32,
    bpm_millis: u32,
    seed: u64,
) -> Result<OfflineRenderReport, MidiNoteError> {
    let schedule = engine_schedule(notes, track);
    let mut report = render_to_wav(
        host,
        &schedule,
        path,
        frames,
        channels,
        sample_rate,
        bpm_millis,
        seed,
    )?;
    report.notes = notes.len();
    Ok(report)
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
#[derive(Debug, Clone, PartialEq)]
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
    /// 离线渲染的参数非法（`frames == 0`）。
    Empty(String),
    /// 引擎重建失败（`EngineHost::reload`）。
    Engine(String),
    /// WAV 量化 / 落盘失败（原样携带 `export_wav` 的裁决）。
    Wav(crate::export_wav::WavExportError),
}

impl MidiNoteError {
    /// 这一档失败的退出码（与 `--midi-in` / `--import-midi` 同一桶 = [`crate::cli::EXIT_OPEN`]）。
    #[must_use]
    pub const fn exit_code(&self) -> u8 {
        match self {
            // ⭐ 落盘类 ⇒ 与 `--export-wav` 同一档；其余 ⇒ 与 `--midi-in` / `--import-midi` 同一桶。
            Self::Wav(_) => crate::cli::EXIT_EXPORT,
            _ => crate::cli::EXIT_OPEN,
        }
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
            Self::Empty(detail) => write!(formatter, "离线渲染的参数非法: {detail}"),
            Self::Engine(detail) => write!(formatter, "引擎重建失败: {detail}"),
            Self::Wav(error) => write!(formatter, "WAV 写出失败: {error}"),
        }
    }
}

impl core::error::Error for MidiNoteError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Wav(error) => Some(error),
            _ => None,
        }
    }
}

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

    /// 判据 ④（**接线登记**，**文本钉住**）：⭐ 引擎侧**确有**两条 `EngineEvent` 音符分支 ＋
    /// `SynthEngine::note_on/note_off`，⭐ 且 **app 侧走的是它**（`--midi-in-render` 经
    /// [`render_to_wav`]）。
    ///
    /// ⚠ **这条判据的历史价值**：`eca718ab` 之前它钉的是"**未**接线"（事件进环只会全静音），
    /// 并在引擎接线的那一刻**变红提醒**（实测 panic 在 `src/midi_notes.rs:625`）；
    /// 现在翻成"**已**接线"，继续钉住**当前事实** —— 引擎把两条分支摘掉、或 app 侧把
    /// `--midi-in-render` 改回**不经环**的路径，本判据都会红。
    #[test]
    fn the_engine_event_note_path_is_wired_to_the_synth() {
        let runtime = include_str!("../../yeban-engine/src/rt.rs");
        let synth = include_str!("../../yeban-engine/src/synth.rs");

        // (a) 出队闭包里**确有**音符分支。
        let drain = runtime
            .split("events.drain_with(")
            .nth(1)
            .and_then(|rest| rest.split("});").next())
            .expect("必须能找到事件出队闭包");
        assert!(
            drain.contains("EngineEvent::NoteOn") && drain.contains("EngineEvent::NoteOff"),
            "引擎的出队闭包必须消费音符事件（⛔ 否则环里的音符不会发声）"
        );
        // (b) 合成器确有音符触发入口。
        assert!(
            synth.contains("pub fn note_on") && synth.contains("pub fn note_off"),
            "`SynthEngine` 必须提供 `note_on` / `note_off`"
        );
        // (c) ⭐ **app 侧走的是它**：本模块的渲染入口把事件写进环并推进引擎。
        let mine = include_str!("midi_notes.rs");
        assert!(
            mine.contains("host.send_engine_events(&batch)") && mine.contains("host.render_into("),
            "离线渲染必须经 `EngineHost::send_engine_events` + `render_into`（环 ⇒ 引擎）"
        );
        // (d) ⭐ CLI 的 `--midi-in-render` 走的是本模块的环路径（⛔ 不是别的路）。
        assert!(
            include_str!("cli.rs").contains("crate::midi_notes::render_notes_to_wav("),
            "`--midi-in-render` 必须经 `midi_notes::render_notes_to_wav()`（环路径）"
        );
    }

    /// 判据 ⑤（**调度表**）：⭐ 音符 ⇒ 事件表：每个音符两条，按 tick 升序，
    /// 同一 tick 上 **NoteOff 先于 NoteOn**（重触发不允许两个声部打架）。
    #[test]
    fn the_engine_schedule_orders_offs_before_ons_at_the_same_tick() {
        let track = EntityId::new();
        let notes = vec![
            CapturedNote {
                pitch: 60,
                velocity: 100,
                start_tick: 0,
                end_tick: 480,
            },
            CapturedNote {
                pitch: 60,
                velocity: 90,
                start_tick: 480,
                end_tick: 960,
            },
        ];
        let schedule = engine_schedule(&notes, track);
        assert_eq!(schedule.len(), 4, "两条音符 ⇒ 4 条事件");
        assert_eq!(schedule[0].0, 0);
        assert!(matches!(
            schedule[0].1,
            EngineEvent::NoteOn {
                pitch: 60,
                velocity: 100,
                ..
            }
        ));
        assert_eq!(schedule[1].0, 480);
        assert!(
            matches!(schedule[1].1, EngineEvent::NoteOff { pitch: 60, .. }),
            "同一 tick 必须先关旧声部: {:?}",
            schedule[1]
        );
        assert!(
            matches!(schedule[2].1, EngineEvent::NoteOn { velocity: 90, .. }),
            "再开新声部: {:?}",
            schedule[2]
        );
        assert_eq!(schedule[3].0, 960);
        assert!(matches!(schedule[3].1, EngineEvent::NoteOff { .. }));
    }

    /// 判据 ⑥（**时间口径**）：⭐ tick↔帧换算是**整数**且互为逆式；速度变了帧/tick 跟着变。
    #[test]
    fn the_tick_frame_conversion_is_integer_and_round_trips() {
        // 48 kHz / 120 BPM ⇒ 一个 4 分音符（960 tick）= 0.5 s = 24000 帧 ⇒ 25 帧/tick。
        assert_eq!(frames_per_tick(48_000, 120_000), 25);
        assert_eq!(ticks_to_frames(960, 48_000, 120_000), 24_000);
        assert_eq!(frames_to_ticks(24_000, 48_000, 120_000), 960);
        for tick in [0_u64, 1, 7, 960, 3_840] {
            let frames = ticks_to_frames(tick, 48_000, 120_000);
            assert_eq!(frames_to_ticks(frames, 48_000, 120_000), tick, "往返一致");
        }
        assert_eq!(frames_per_tick(48_000, 240_000), 12, "240 BPM 是一半");
        assert_eq!(frames_per_tick(48_000, 0), 25, "坏输入兜底 120 BPM");
        assert_eq!(frames_per_tick(0, 120_000), 0);
    }

    /// 判据 ⑦（**时间口径**）：⭐ 本模块的 `bpm_millis` 与 `bridge` 投影**逐值相同**
    /// （⛔ 不许两个模块各写一份速度口径）。
    #[test]
    fn bpm_millis_agrees_with_the_bridge_projection() {
        for bpm in [60.0_f64, 120.0, 128.5, 174.999] {
            let project = YebanProjectV1 {
                bpm,
                ..yeban_model::samples::filled_project()
            };
            let view = crate::bridge::ViewState::from_project(&project).expect("投影");
            assert_eq!(bpm_millis(bpm), view.bpm_millis, "bpm={bpm} 口径必须一致");
        }
        assert_eq!(bpm_millis(f64::NAN), 120_000);
        assert_eq!(bpm_millis(0.0), 120_000);
    }

    /// 判据 ⑧（**声音那一半的反向对照**）：⭐ 空调度表 / 0 帧必须**如实拒绝**
    /// （⛔ 不渲染一个空 WAV 冒充成功）。
    #[test]
    fn an_empty_schedule_is_refused_instead_of_rendering_silence() {
        let mut host = EngineHost::new();
        assert_eq!(
            render_engine_offline(&mut host, &[], 256, 2, 48_000, 120_000).unwrap_err(),
            MidiNoteError::NoNotes
        );
        let track = EntityId::new();
        let schedule = engine_schedule(
            &[CapturedNote {
                pitch: 60,
                velocity: 100,
                start_tick: 0,
                end_tick: 480,
            }],
            track,
        );
        assert!(matches!(
            render_engine_offline(&mut host, &schedule, 0, 2, 48_000, 120_000).unwrap_err(),
            MidiNoteError::Empty(_)
        ));
        assert_eq!(MidiNoteError::NoNotes.exit_code(), crate::cli::EXIT_OPEN);
    }
}
