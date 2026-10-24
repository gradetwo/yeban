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
//!
//! ## 口径（⭐ 本导入**只**消费什么、**不**消费什么）
//!
//! ⚠ 本节是**读数**，⛔ 不是评判：每一条都由 `grep` / 类型可从本文件的实现复算。
//!
//! 1. **音符映射只消费 [`ParsedMidi::notes`] / [`ParsedMidi::lyrics`] / [`ParsedMidi::ppq`]**；
//!    ⭐ `#318` 的丢失读数另**只读** `format` / `tracks` / `aux_events`（⛔ 不参与映射）。
//!    由此可复算：`grep -n 'parsed\.' crates/yeban-app/src/import_midi.rs` 里
//!    ⛔ 不出现 `parsed.tempos`（tempo/拍号是作为 `aux_events` 的计数被报成**丢**，
//!    ⛔ 不是被消费）。
//! 2. **tempo map 不参与映射**：`ParsedMidi::tempos`（[`yeban_midi::midi::MidiTempo`]）
//!    在本文件的**生产**代码里⛔ 一处都不出现；它只作为**判据夹具**出现在 `#[cfg(test)]` 里。
//!    ⇒ 音符的**绝对 tick**（以及音符之间的相对 tick）按 `ppq` 换算后**原样**落到工程上，
//!    ⛔ 不做任何与速度有关的换算。
//! 3. **不量化**：[`scale_ticks`] 是**纯比例**换算（`tick × MODEL_PPQ ÷ ppq`），
//!    ⛔ 不吸附到工程网格；`ppq` 除不尽时按整数除法**截断**。
//! 4. **单一目标**：全部音符进**同一个**音轨与**同一个**片段 —— 目标由
//!    [`MidiImportOptions`] 决定（`None` 时取第一条非 master 音轨、该轨的第一个摆放）。
//!    ⛔ 不按通道 / ⛔ 不按文件里的轨道拆分。
//! 5. **歌词只做"唯一才归属"**（⭐ `#317`）：按**文件 tick** 看，恰 1 颗**起点**音符
//!    ∧ 恰 1 条非空严格 UTF-8 歌词 ⇒ 写进那颗音符的 `syllable`；其余逐条计入
//!    [`MidiLyricCounts::dropped`]。⛔ 按文件序猜和弦、⛔ 一颗词复制给多颗、
//!    ⛔ 认"正在发声"的音符。空 `FF 05` 与非 UTF-8 分别计入 `empty` / `non_utf8`。
//! 6. **支持的输入范围**：⛔ 本文件不复述"支持哪些格式" —— 那是
//!    [`yeban_midi::midi::parse_smf`] 及其错误变体的事（⭐ 指路，⛔ 不手抄）。
//! 7. **其余事件只报读数、⛔ 不新增支持**（⭐ `#318`）：[`ParsedMidi::aux_events`] 把
//!    24 类具名事件逐一计数（[`MidiAuxEventKind`]），本文件把它们**全部**报成
//!    `dropped`（今天 `dropped == read`）。报告字段：`format=` / `file-tracks=` /
//!    `track-partitions-dropped=` / `channels=`（上下文，总是打印）＋ 每类
//!    `<kind>-read=N <kind>-dropped=N`（仅 `read>0`）＋ `note-channel-*` /
//!    `note-off-velocity-*`（按音符计）。
//!    ⚠ **"值一样"不等于"没丢"**：`tempo` 的数值即使恰好等于工程 `bpm`，它也仍是丢
//!    （没被消费）。支持某一类时把它的 `dropped` 改成 0，并**改掉**判据；⛔ 不许删判据。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use yeban_midi::midi::{MidiAuxEventKind, MidiFormat, ParsedMidi, parse_smf};
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

/// ⭐ `#317`：一次导入的歌词读数（**唯一**计算点在 [`plan_midi_import`]）。
///
/// 分桶互不重叠，且恒有 **`read == attached + dropped`**：
/// - `read`：非空、严格 UTF-8 解码成功的 `FF 05` 条数（可归属的候选）；
/// - `attached`：唯一归属到一颗音符的条数（该 tick **恰 1 颗起点音符** ∧ 恰 1 条候选歌词）；
/// - `dropped`：没有归属的候选条数（和弦 / 一 tick 多词 / 无音符）；
/// - `non_utf8`：负载不是合法 UTF-8 的 `FF 05` 条数（⛔ lossy、⛔ 猜）；
/// - `empty`：负载为空的 `FF 05` 条数（没有音节可写）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MidiLyricCounts {
    /// 可归属的候选条数（非空 + 严格 UTF-8）。
    pub read: usize,
    /// 唯一归属的条数。
    pub attached: usize,
    /// 未归属的候选条数。
    pub dropped: usize,
    /// 非 UTF-8 的 `FF 05` 条数。
    pub non_utf8: usize,
    /// 空 `FF 05` 条数。
    pub empty: usize,
}

impl MidiLyricCounts {
    /// 自检：`read == attached + dropped`（分桶既不重叠也不漏）。
    #[must_use]
    pub const fn is_consistent(self) -> bool {
        self.read == self.attached + self.dropped
    }
}

/// ⭐ `#318`：一类丢失的**两列读数**。
///
/// `read` 是文件里读到的条数（判据另按字节独立数一遍做控制臂）；
/// `dropped` 是导入路径**没有**落进工程的条数。今天每一类都 `dropped == read`。
///
/// ⚠ **"值一样"不等于"没丢"**：`tempo` 的数值即使恰好等于工程 `bpm`，它也**仍是丢**
/// —— 判据是"**有没有被消费**"，⛔ 不是"数值一不一样"。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MidiLossCount {
    /// 文件里读到的条数。
    pub read: usize,
    /// 导入路径没有落进工程的条数。
    pub dropped: usize,
}

impl MidiLossCount {
    /// 两列的自然约束：`dropped <= read`。
    #[must_use]
    pub const fn is_consistent(self) -> bool {
        self.dropped <= self.read
    }

    /// 今天**全损**：读到多少就丢多少。
    ///
    /// ⚠ 这是一条**会过期**的判据：支持某一类时，必须把那一类的 `dropped` 改成
    /// `<= read` 的真值（通常 0），并**改掉** `dropped == read` 的断言；
    /// ⛔ 不许把判据删掉 —— 它过期时应当自己说出来。
    #[must_use]
    pub const fn is_total_loss(self) -> bool {
        self.dropped == self.read
    }
}

/// ⭐ `#318`：一次导入的**其余丢失读数**（音符 / 歌词以外）。
///
/// ## 今天为什么每一项都是"丢"（⛔ 不是"已处理"）
///
/// - `tempo` / 拍号：工程的 `bpm` / `time_signature` 是唯一权威 ⇒ 文件里的值
///   **不被消费**。⚠ **值恰好等于工程值也仍是丢**：判据是"有没有被消费"，
///   ⛔ 不是"数值一不一样"。
/// - `FF 03` 轨名 / `FF 06` marker / `FF 59` 调号 / CC / 弯音 / 触后 /
///   Program Change / SysEx / Escape / 其余 meta：模型或导入映射没有落点 ⇒ 全丢。
/// - 轨分区：全部音符进**同一个**音轨与同一个片段 ⇒ 文件 N 条 `MTrk` 只剩 1 个落点，
///   `N-1` 个分区丢失。
/// - 音符通道：[`MidiNote`] 没有 channel 字段 ⇒ 每颗音符的通道值都丢
///   （只作为解析排序键残留）。
/// - 释放力度：`NoteOff` 的 velocity 被解析器忽略 ⇒ 每颗被关闭音符都丢。
///
/// `format` / `file_tracks` / `distinct_channels` 是**上下文读数**（⛔ 损失），
/// 但报告**总是**打印它们（值为 0 也不是"没有这个读数"）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiImportLosses {
    /// 文件格式（读数；`format 2` / SMPTE 在解析层已**响亮拒绝**）。
    pub format: MidiFormat,
    /// 文件里的 `MTrk` 条数。
    pub file_tracks: usize,
    /// 被合并进单目标的轨分区条数 = `file_tracks - 1`。
    pub track_partitions_dropped: usize,
    /// 音符里出现过的**不同**通道值个数（读数，⛔ 不是损失）。
    pub distinct_channels: usize,
    /// 每颗音符的通道值（read = 音符数）。
    pub note_channel_values: MidiLossCount,
    /// 每颗被关闭音符的释放力度（read = 音符数）。
    pub note_off_velocity_values: MidiLossCount,
    /// 24 类 aux 事件，逐类 read/dropped（今天 `dropped == read`）。
    pub aux_events: BTreeMap<MidiAuxEventKind, MidiLossCount>,
}

impl MidiImportLosses {
    /// 自检：每条 `dropped <= read`；`track_partitions_dropped + 1 == file_tracks`。
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.file_tracks >= 1
            && self.track_partitions_dropped + 1 == self.file_tracks
            && self.note_channel_values.is_consistent()
            && self.note_off_velocity_values.is_consistent()
            && self.aux_events.values().all(|count| count.is_consistent())
    }

    /// 今天**没有一类**被支持 ⇒ 每条 `dropped == read`（含音符通道 / 释放力度）。
    ///
    /// ⚠ 这条判据**会过期**：支持某一类时它应该红，提醒把那类的 `dropped` 改成 0；
    /// ⛔ 不是把这条判据删掉。
    #[must_use]
    pub fn is_total_loss(&self) -> bool {
        self.note_channel_values.is_total_loss()
            && self.note_off_velocity_values.is_total_loss()
            && self.aux_events.values().all(|count| count.is_total_loss())
    }

    /// 24 类 aux 事件的 read 条数之和（⛔ 不含音符通道 / 释放力度）。
    #[must_use]
    pub fn read_events(&self) -> usize {
        self.aux_events.values().map(|count| count.read).sum()
    }

    /// 24 类 aux 事件的 dropped 条数之和（⛔ 不含音符通道 / 释放力度）。
    #[must_use]
    pub fn dropped_events(&self) -> usize {
        self.aux_events.values().map(|count| count.dropped).sum()
    }

    /// 24 类 aux 事件的逐类字段串（`<kind>-read=N <kind>-dropped=N …`）。
    ///
    /// ⭐ `#319`：这是**唯一**的逐类格式化处 —— [`MidiImportReport::summary`]（CLI）
    /// 与 GUI 的 `loss_detail="…"` 字段**共用**它 ⇒ ⛔ 不是第二份格式化。
    ///
    /// 只有 `read > 0` 的类才打印（`#318` 起的 CLI 口径）；每类前带一个空格 ⇒
    /// 空表返回空串，非空表可直接拼在上一字段之后。字段里的 `=` / 空格在 GUI 回执里
    /// 整体落在**引号内**，不参与回执语法（同 `clipping_note` 先例）。
    #[must_use]
    pub fn aux_fields(&self) -> String {
        let mut out = String::new();
        for (kind, count) in &self.aux_events {
            if count.read == 0 {
                continue;
            }
            out.push_str(&format!(
                " {}-read={} {}-dropped={}",
                kind.key(),
                count.read,
                kind.key(),
                count.dropped
            ));
        }
        out
    }

    /// 明细字段的完整值（GUI 回执的 `loss_detail="…"`）：4 项上下文 ＋ 按音符 4 项
    /// ＋ [`Self::aux_fields`] 的 24 类逐类读数。
    ///
    /// ⭐ `#319`：CLI 的 `imported-midi:` 行尾部也**直接拼这一串**
    /// （[`MidiImportReport::summary`]）⇒ 两个出口的明细来自**同一个格式化函数**。
    #[must_use]
    pub fn loss_detail(&self) -> String {
        format!(
            "format={} file-tracks={} track-partitions-dropped={} channels={} \
             note-channel-read={} note-channel-dropped={} note-off-velocity-read={} \
             note-off-velocity-dropped={}{}",
            self.format.number(),
            self.file_tracks,
            self.track_partitions_dropped,
            self.distinct_channels,
            self.note_channel_values.read,
            self.note_channel_values.dropped,
            self.note_off_velocity_values.read,
            self.note_off_velocity_values.dropped,
            self.aux_fields(),
        )
    }

    /// 损失摘要的**可见短句**（GUI 专用行的文本；唯一写者是
    /// `host::apply_module_receipt_with_alerts`）。
    ///
    /// 单位逐项命名：`类` = `dropped > 0` 的 aux 事件类数；`条` = 24 类的 `dropped` 之和；
    /// `通道值` / `释放力度` = 按音符计的 `dropped`。⚠ 不含歌词（`#317` 的读数，本单不做）。
    /// 无 aux 丢失时省略"0 类 / 0 条"那一截，只留按音符的两项（仍然如实说）。
    #[must_use]
    pub fn loss_summary(&self) -> String {
        let classes = self
            .aux_events
            .values()
            .filter(|count| count.dropped > 0)
            .count();
        let events = self.dropped_events();
        if events == 0 {
            format!(
                "⚠ 导入丢失：通道值 {}、释放力度 {}",
                self.note_channel_values.dropped, self.note_off_velocity_values.dropped
            )
        } else {
            format!(
                "⚠ 导入丢失：{classes} 类 / {events} 条；通道值 {}、释放力度 {}",
                self.note_channel_values.dropped, self.note_off_velocity_values.dropped
            )
        }
    }
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
    /// ⭐ `#317`：歌词的读取／归属／丢弃读数。
    pub lyrics: MidiLyricCounts,
    /// ⭐ `#318`：音符/歌词以外的**逐类丢失读数**（⛔ 不沉默地少做）。
    pub losses: MidiImportLosses,
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
    ///   `<kind>-read=N <kind>-dropped=N`（键名见 [`MidiAuxEventKind::key`]）；
    /// - **两个按音符计的属性**：`note-channel-*` / `note-off-velocity-*`
    ///   （`read == dropped == notes`，今天）。
    ///
    /// ⚠ `dropped == read` 今天对每一类成立（**无任何真支持**）。支持某一类时改它的
    /// `dropped` 并改掉判据；⛔ 不许删判据。
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
            "imported-midi: path={} notes={} lyrics={} attached={} dropped={} non-utf8={} \
             empty={} track={} clip={} commit={} origin={origin} {}",
            self.path
                .as_ref()
                .map_or_else(|| "<bytes>".to_owned(), |path| path.display().to_string()),
            self.notes,
            self.lyrics.read,
            self.lyrics.attached,
            self.lyrics.dropped,
            self.lyrics.non_utf8,
            self.lyrics.empty,
            self.track_id.to_canonical_string(),
            self.clip_id.to_canonical_string(),
            self.commit.to_canonical_string(),
            self.losses.loss_detail(),
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
/// ⚠ **只消费 `parsed.notes` / `parsed.lyrics` / `parsed.ppq`**（⭐ 可由本函数体
/// `grep 'parsed\.'` 复算）；[`ParsedMidi::tempos`] 与 [`ParsedMidi::format`] ⛔ 不参与映射
/// —— 完整口径见本模块文档的"口径"一节。
///
/// ## ⭐ `#317` 歌词归属：**唯一才归属**
///
/// 一条歌词只在**同时**满足下面两条时写进对应 [`MidiNote::syllable`]：
///
/// 1. 该 **tick 恰有 1 颗起点音符**（`note.start_tick == lyric.tick`）；
/// 2. 该 tick **恰有 1 条非空、严格 UTF-8 的歌词**。
///
/// ⚠ 语义边界（`#316` 实测的结论，写在这里而不只写在报告里）：
/// **只认起点 == tick 的音符、⛔ 不认正在发声（跨过该 tick）的音符。**
/// 因此 `filled` 的 4/4 全部归属；4 颗音符同一 tick 的 4 条词**一条都不归属**
/// （⛔ 按文件序配对 —— `#316` 实测文本序与音高序不同，配对会 4/4 全错）；
/// 4 颗音符只有 1 条词也**不归属**（⛔ 一颗词复制给四颗 = 零膨胀的反面）。
/// 未归属的候选**逐条计入** [`MidiLyricCounts::dropped`]，⛔ 不沉默。
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

    // ⭐ `#317`：按**文件 tick** 建两张计数表（⛔ `scale_ticks` 之后 —— 归属是文件里的
    // 时间关系，不是模型的 960 PPQ 关系）。只数**起点**，⛔ 不数正在发声的音符。
    let mut note_count_at: BTreeMap<u64, usize> = BTreeMap::new();
    for note in &parsed.notes {
        *note_count_at.entry(note.start_tick).or_insert(0) += 1;
    }
    let mut lyric_count_at: BTreeMap<u64, usize> = BTreeMap::new();
    for lyric in &parsed.lyrics {
        *lyric_count_at.entry(lyric.tick).or_insert(0) += 1;
    }
    // 可归属的 tick → 文本。两个恰为 1 才进表；其余一律留在 `dropped`。
    let mut attachable: BTreeMap<u64, &str> = BTreeMap::new();
    for lyric in &parsed.lyrics {
        if note_count_at.get(&lyric.tick) == Some(&1) && lyric_count_at.get(&lyric.tick) == Some(&1)
        {
            attachable.insert(lyric.tick, lyric.text.as_str());
        }
    }

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
        if let Some(text) = attachable.get(&note.start_tick) {
            model_note.syllable = Some((*text).to_owned());
        }
        Op::AddNote {
            track_id,
            clip_id,
            note: model_note,
        }
    }));
    let notes = ops.len();
    let lyrics = MidiLyricCounts {
        read: parsed.lyrics.len(),
        attached: attachable.len(),
        dropped: parsed.lyrics.len() - attachable.len(),
        non_utf8: parsed.non_utf8_lyrics,
        empty: parsed.empty_lyrics,
    };
    // 分桶自检：既不重叠也不漏（`read == attached + dropped`）。
    debug_assert!(
        lyrics.is_consistent(),
        "歌词分桶必须满足 read == attached + dropped: {lyrics:?}"
    );

    // ⭐ `#318`：把解析器数到的每一类 aux 事件原样变成 read/dropped 两列。
    // ⚠ 今天**没有任何一类被消费** ⇒ `dropped == read` 对每一类都成立。
    // ⚠ **"值一样"不等于"没丢"**：`tempo` 即使数值恰好等于工程 bpm 也仍是丢
    // （`project.bpm` 是唯一权威，文件里的值没被消费）。支持某一类时把它的
    // `dropped` 改成 0，并**改掉** `dropped == read` 的判据；⛔ 不许删判据。
    let mut aux_events: BTreeMap<MidiAuxEventKind, MidiLossCount> = BTreeMap::new();
    for kind in MidiAuxEventKind::ALL {
        let read = parsed.aux_events.count(kind);
        aux_events.insert(
            kind,
            MidiLossCount {
                read,
                dropped: read,
            },
        );
    }
    let distinct_channels = parsed
        .notes
        .iter()
        .map(|note| note.channel)
        .collect::<BTreeSet<u8>>()
        .len();
    let losses = MidiImportLosses {
        format: parsed.format,
        file_tracks: parsed.tracks,
        track_partitions_dropped: parsed.tracks.saturating_sub(1),
        distinct_channels,
        // 每颗音符都带一个通道值，而 `MidiNote` 没有 channel 字段 ⇒ 全丢。
        note_channel_values: MidiLossCount {
            read: notes,
            dropped: notes,
        },
        // 每颗音符都有一条关闭事件，它的释放力度被解析器忽略 ⇒ 全丢。
        note_off_velocity_values: MidiLossCount {
            read: notes,
            dropped: notes,
        },
        aux_events,
    };
    debug_assert!(
        losses.is_consistent(),
        "丢失读数必须满足 dropped <= read 且轨分区 + 1 == file_tracks: {losses:?}"
    );
    debug_assert!(
        losses.is_total_loss(),
        "今天没有一类被支持 ⇒ 每条 dropped == read（支持一类时改这里，⛔ 别删判据）: {losses:?}"
    );
    Ok(MidiImportPlan {
        ops,
        notes,
        track_id,
        clip_id,
        ppq: parsed.ppq,
        lyrics,
        losses,
    })
}

/// 选目标：第一条非 master 的音轨 ＋ 它的第一个摆放。
///
/// ⚠ `pub(crate)`：`crate::midi_notes` 的 `--midi-in` 写音符走**同一条**规则
/// （⛔ 不许两处各写一份"第一条非 master 音轨"）。
pub(crate) fn destination(
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
    use yeban_midi::midi::{
        MidiAuxEventCounts, MidiAuxEventKind, MidiFormat, MidiTempo, ParsedLyric, ParsedNote,
    };

    /// 造一份**直接可用的** `ParsedMidi`（⛔ 不需要真的编码 SMF：本判据测的是**映射**）。
    fn parsed(ppq: u16, notes: Vec<ParsedNote>) -> ParsedMidi {
        parsed_with_lyrics(ppq, notes, Vec::new(), 0, 0)
    }

    /// 同上，另带歌词读数（⛔ 不经 `parse_smf`，本判据只测归属）。
    fn parsed_with_lyrics(
        ppq: u16,
        notes: Vec<ParsedNote>,
        lyrics: Vec<ParsedLyric>,
        non_utf8_lyrics: usize,
        empty_lyrics: usize,
    ) -> ParsedMidi {
        ParsedMidi {
            format: MidiFormat::SingleTrack,
            ppq,
            tracks: 1,
            aux_events: MidiAuxEventCounts::default(),
            tempos: Vec::<MidiTempo>::new(),
            notes,
            lyrics,
            non_utf8_lyrics,
            empty_lyrics,
        }
    }

    fn lyric(tick: u64, text: &str) -> ParsedLyric {
        ParsedLyric {
            tick,
            text: text.to_owned(),
        }
    }

    fn syllables(plan: &MidiImportPlan) -> Vec<(u64, Option<String>)> {
        let mut rows: Vec<(u64, Option<String>)> = plan
            .ops
            .iter()
            .map(|op| match op {
                Op::AddNote { note, .. } => (note.start_tick, note.syllable.clone()),
                other => panic!("必须是 AddNote: {other:?}"),
            })
            .collect();
        rows.sort_by_key(|row| row.0);
        rows
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

    /// ⭐ `#317` 判据 a（`filled` 形状）：一 tick 一颗音符、一条词 ⇒ **4/4 全部归属**，
    /// `syllable` 逐颗落到**同一 tick** 的那颗音符上（按 `do/re/mi/fa` 的文件 tick）。
    #[test]
    fn lyrics_attach_when_each_tick_has_exactly_one_note_and_one_lyric() {
        let project = yeban_model::samples::filled_project();
        let plan = plan_midi_import(
            &project,
            &parsed_with_lyrics(
                960,
                vec![
                    note(60, 100, 0, 480),
                    note(64, 100, 960, 1440),
                    note(67, 100, 1920, 2400),
                    note(72, 100, 2880, 3360),
                ],
                vec![
                    lyric(0, "do"),
                    lyric(960, "re"),
                    lyric(1920, "mi"),
                    lyric(2880, "fa"),
                ],
                0,
                0,
            ),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(
            plan.lyrics,
            MidiLyricCounts {
                read: 4,
                attached: 4,
                dropped: 0,
                non_utf8: 0,
                empty: 0,
            },
            "4 条候选、4 条唯一归属"
        );
        assert!(plan.lyrics.is_consistent(), "read == attached + dropped");
        assert_eq!(
            syllables(&plan),
            vec![
                (0, Some("do".to_owned())),
                (960, Some("re".to_owned())),
                (1920, Some("mi".to_owned())),
                (2880, Some("fa".to_owned())),
            ],
            "音节必须落到同一 tick 的那颗音符上"
        );
    }

    /// ⭐ `#317` 判据 b（和弦正对照，`#316` 实测的形状）：**同一 tick 4 颗起点音符 ＋
    /// 4 条词** ⇒ 一条都不归属（⛔ 按文件序一一配对 —— 文本序与音高序不同，4/4 会全错），
    /// 4 条**逐条**计入 `dropped`；⛔ 零错配、⛔ 零膨胀。
    #[test]
    fn a_chord_with_four_lyrics_attaches_none_and_reports_all_four_as_dropped() {
        let project = yeban_model::samples::filled_project();
        let plan = plan_midi_import(
            &project,
            &parsed_with_lyrics(
                960,
                vec![
                    note(64, 100, 960, 1440),
                    note(67, 100, 960, 1440),
                    note(72, 100, 960, 1440),
                    note(76, 100, 960, 1440),
                ],
                vec![
                    lyric(960, "laa"),
                    lyric(960, "re"),
                    lyric(960, "si"),
                    lyric(960, "sol"),
                ],
                0,
                0,
            ),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(
            plan.lyrics,
            MidiLyricCounts {
                read: 4,
                attached: 0,
                dropped: 4,
                non_utf8: 0,
                empty: 0,
            },
            "和弦 ⇒ 0 归属、4 条 dropped"
        );
        assert!(plan.lyrics.is_consistent(), "read == attached + dropped");
        assert_eq!(
            syllables(&plan)
                .iter()
                .filter(|row| row.1.is_some())
                .count(),
            0,
            "⛔ 猜：4 颗音符一条都不许拿到音节"
        );
    }

    /// ⭐ `#317` 判据 c（单颗带词的和弦）：4 颗起点音符只有 1 条词 ⇒ **也不归属**
    /// （那个 tick 有 4 颗音符 ⇒ 唯一性不成立），`dropped` 报 **1**；⛔ 复制给四颗。
    #[test]
    fn one_lyric_on_a_four_note_chord_is_dropped_instead_of_copied() {
        let project = yeban_model::samples::filled_project();
        let plan = plan_midi_import(
            &project,
            &parsed_with_lyrics(
                960,
                vec![
                    note(64, 100, 960, 1440),
                    note(67, 100, 960, 1440),
                    note(72, 100, 960, 1440),
                    note(76, 100, 960, 1440),
                ],
                vec![lyric(960, "laa")],
                0,
                0,
            ),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(
            plan.lyrics,
            MidiLyricCounts {
                read: 1,
                attached: 0,
                dropped: 1,
                non_utf8: 0,
                empty: 0,
            },
            "1 条词 + 4 颗音符 ⇒ 不归属、dropped=1"
        );
        assert_eq!(
            syllables(&plan)
                .iter()
                .filter(|row| row.1.is_some())
                .count(),
            0,
            "⛔ 不许把一颗词复制给四颗音符（零膨胀）"
        );
    }

    /// ⭐ `#317` 语义边界（`#316`）：**只认起点 == tick 的音符、⛔ 不认正在发声的音符**。
    /// 一颗从 0 响到 960 的音符跨过了 tick 480，但 480 上没有任何音符**起点** ⇒ dropped=1。
    #[test]
    fn a_note_that_only_spans_the_lyric_tick_is_not_its_owner() {
        let project = yeban_model::samples::filled_project();
        let plan = plan_midi_import(
            &project,
            &parsed_with_lyrics(
                960,
                vec![note(60, 100, 0, 960)],
                vec![lyric(480, "mid")],
                0,
                0,
            ),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(plan.lyrics.read, 1);
        assert_eq!(plan.lyrics.attached, 0, "⛔ 正在发声 ≠ 起点");
        assert_eq!(plan.lyrics.dropped, 1);
        assert!(syllables(&plan).iter().all(|row| row.1.is_none()));
    }

    /// ⭐ `#317` 分桶自检：`read == attached + dropped` 恒成立；空／非 UTF-8 的计数
    /// **原样**从解析层传进报告（⛔ 被并进 dropped、⛔ 静默丢）。
    #[test]
    fn the_lyric_buckets_are_disjoint_and_read_equals_attached_plus_dropped() {
        let project = yeban_model::samples::filled_project();
        let plan = plan_midi_import(
            &project,
            &parsed_with_lyrics(
                960,
                vec![note(60, 100, 0, 480), note(64, 100, 960, 1440)],
                vec![lyric(0, "do"), lyric(960, "re"), lyric(960, "sol")],
                2,
                1,
            ),
            &MidiImportOptions::default(),
        )
        .expect("必须有目标");
        assert_eq!(
            plan.lyrics,
            MidiLyricCounts {
                read: 3,
                attached: 1,
                dropped: 2,
                non_utf8: 2,
                empty: 1,
            }
        );
        assert!(plan.lyrics.is_consistent(), "read == attached + dropped");
        assert_eq!(
            plan.lyrics.read,
            plan.lyrics.attached + plan.lyrics.dropped,
            "恒等式必须逐字成立"
        );
    }

    /// ⭐ `#318`：报告把**每一类** aux 事件都成对报出（read/dropped），且今天
    /// 每类 `dropped == read` —— 没有一类被冒充成"已处理"。
    ///
    /// 控制臂：计数不是零（tempo=2、marker=1…），所以"全损"不是"没有数据"造出来的
    /// 恒真；`read_events() == 7` 是夹具里**真的**有 7 条 event 的读数。
    #[test]
    fn the_loss_report_pairs_read_and_dropped_for_every_aux_kind() {
        let project = yeban_model::samples::filled_project();
        let mut aux = MidiAuxEventCounts::default();
        aux.record(MidiAuxEventKind::Tempo);
        aux.record(MidiAuxEventKind::Tempo);
        aux.record(MidiAuxEventKind::TrackName);
        aux.record(MidiAuxEventKind::Marker);
        aux.record(MidiAuxEventKind::ControlChange);
        aux.record(MidiAuxEventKind::PitchBend);
        aux.record(MidiAuxEventKind::UnknownMeta);
        let parsed = ParsedMidi {
            format: MidiFormat::Parallel,
            ppq: 960,
            tracks: 3,
            aux_events: aux,
            tempos: Vec::new(),
            notes: vec![
                ParsedNote {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    channel: 1,
                    key: 48,
                    velocity: 110,
                    start_tick: 0,
                    end_tick: 960,
                },
            ],
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("必须有目标");
        let losses = &plan.losses;
        assert_eq!(losses.format, MidiFormat::Parallel);
        assert_eq!(losses.file_tracks, 3);
        assert_eq!(
            losses.track_partitions_dropped, 2,
            "3 条 MTrk 进 1 个目标 ⇒ 2 个分区丢失"
        );
        assert_eq!(losses.distinct_channels, 2, "音符里出现过 {{0,1}} 两个通道");
        assert_eq!(
            losses.note_channel_values,
            MidiLossCount {
                read: 2,
                dropped: 2,
            },
            "每颗音符一个通道值，模型没有字段 ⇒ 全丢"
        );
        assert_eq!(
            losses.note_off_velocity_values,
            MidiLossCount {
                read: 2,
                dropped: 2,
            },
            "每颗音符一条关闭事件，释放力度没有字段 ⇒ 全丢"
        );
        assert_eq!(
            losses.aux_events.len(),
            MidiAuxEventKind::COUNT,
            "24 类一个不缺（即使 read=0 也在表里）"
        );
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::Tempo],
            MidiLossCount {
                read: 2,
                dropped: 2,
            }
        );
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 1,
                dropped: 1,
            }
        );
        for (kind, count) in &losses.aux_events {
            assert_eq!(
                count.dropped,
                count.read,
                "{} 今天必须全损（支持它时改这里，⛔ 不是删判据）",
                kind.key()
            );
            assert!(count.is_consistent(), "{}: dropped <= read", kind.key());
        }
        assert!(losses.is_consistent());
        assert!(losses.is_total_loss());
        assert_eq!(losses.read_events(), 7, "夹具里真的 7 条 aux 事件");
        assert_eq!(losses.dropped_events(), 7);
        assert_eq!(plan.notes, 2);
        assert_eq!(plan.lyrics, MidiLyricCounts::default());
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

#[cfg(test)]
mod tempo_pin_tests {
    use super::*;
    use crate::undo::UndoPort;

    /// 造一份**只差 tempo** 的最小 SMF：1 条轨、960 PPQ、一个 C4 音符、⭐ 指定 tempo。
    fn smf_with_tempo(micros: u32) -> Vec<u8> {
        let mut bytes: Vec<u8> = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes()); // 格式 0
        bytes.extend_from_slice(&1u16.to_be_bytes()); // 1 条轨
        bytes.extend_from_slice(&960u16.to_be_bytes()); // 960 PPQ
        let mut track: Vec<u8> = Vec::new();
        // `FF 51 03 <3 字节大端 µs/四分音符>`
        track.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03]);
        track.extend_from_slice(&micros.to_be_bytes()[1..]);
        track.extend_from_slice(&[0x00, 0x90, 0x3C, 0x64]); // note on C4 vel 100
        track.extend_from_slice(&[0x87, 0x40, 0x80, 0x3C, 0x40]); // delta 960: note off
        track.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]); // end of track
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&u32::try_from(track.len()).unwrap().to_be_bytes());
        bytes.extend_from_slice(&track);
        bytes
    }

    /// ⭐ **D2 主判据**：⭐ **导入**一份带**极端**变速的 SMF ⇒ ⭐ **工程的 `bpm` 一字不变**。
    ///
    /// ## 这条判据在什么条件下会红（**有牙**）
    ///
    /// ⭐ ⭐ 有人让导入路径**消费 `parsed.tempos`**、并把值**真的写进** `project.bpm`
    /// ⇒ ⭐ 第二条断言红。⭐ **已实测注入**（把 240 BPM 写进 `bpm` ⇒ 红；见交付报告）。
    ///
    /// ## ⭐ 控制臂（**在同一条判据里**，⛔ 不放别处）
    ///
    /// ⭐ 断言 **音符确实进来了**（`report.notes == 1`）＋ ⭐ **文件确实带的是那个极端 tempo**
    /// （重新解析它读回 250_000）⇒ ⭐ 否则"`bpm` 没变"可能只是因为**导入压根没发生**（恒真）。
    #[test]
    fn importing_a_tempo_changing_smf_does_not_touch_the_projects_bpm() {
        // ⭐ 极端值：250_000 µs/四分音符 = **240 BPM**（与工程侧 90 BPM 明显不同）。
        const EXTREME_MICROS: u32 = 250_000;
        let bytes = smf_with_tempo(EXTREME_MICROS);

        // ⭐ 控制臂（前置）：文件**真的**携带 240 BPM，⛔ 不是"看起来极端"。
        let parsed_file = yeban_midi::midi::parse_smf(&bytes).expect("解析自造 SMF");
        assert_eq!(
            parsed_file.tempos.len(),
            1,
            "⭐ 控制臂前置：文件必须真的带一条 tempo（⛔ 不然后面什么都没测）"
        );
        assert_eq!(
            parsed_file.tempos[0].microseconds_per_quarter,
            Some(EXTREME_MICROS),
            "⭐ 控制臂前置：文件带的必须是那个**极端** tempo"
        );

        // 工程侧：`bpm = 90`（⭐ 与文件里的 240 明显不同 ⇒ 若被导入就会看得出来）。
        let mut project = yeban_model::samples::filled_project();
        project.bpm = 90.0;
        let session =
            crate::undo::UndoSession::open("<判据:tempo 不导入>", "yeban-app", project.clone(), 1)
                .expect("打开会话");
        let port = UndoPort::new(session);

        let report = import_midi_bytes(&port, &project, &bytes, &MidiImportOptions::default(), 10)
            .expect("极端变速的 SMF 必须能导入");

        // ⭐ 控制臂：导入**真的发生了**。
        assert_eq!(
            report.notes, 1,
            "⭐ 控制臂：音符必须真的进来了（⛔ 否则'`bpm` 没变'可能只是没导入）"
        );

        // ⭐ **主断言**：权威工程的 `bpm` 一字不变。
        assert_eq!(
            port.project().bpm,
            90.0,
            "⭐ D2：导入⛔ 不许改工程的 `bpm`（工程的 tempo 是**单一标量** `project.bpm`，\
             而 MIDI tempo map 可携带任意多条带 tick 的变速 ⇒ 若导入就会同时持有**两条真相**）"
        );
        assert_eq!(
            port.project().transport.count_in_bars,
            project.transport.count_in_bars,
            "⭐ 相邻字段（同一 `TransportConfig`）也必须一字不变 —— 证明改的不是别处"
        );
    }
}
