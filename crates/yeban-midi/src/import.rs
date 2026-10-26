//! Standard MIDI File (`.mid`) 的**导入映射**：把一段 SMF 里的音符与元数据映射成
//! 模型的操作序列（`Op`），与 [`crate::export`] 对称。
//!
//! ## 需求规范 ID
//!
//! - `[ARCH-EXT-002]`：外部 DAW 与标准 MIDI 文件互操作规范，支持 SMF 0 / SMF 1 导入。
//! - `[MODEL-AST-001]`：960 PPQ 整数时钟契约，SMF 时间分度换算到模型的 960 PPQ。
//! - `[MODEL-AST-003]`：确定性状态与 BTreeMap 拓扑，事件排序与丢失统计的一致性。
//! - `[MODEL-AST-005]`：MidiNote 核心实体与属性映射（音高、力度、时值、歌词 syllable）。
//!
//! ## 物理架构解耦与零依赖边
//!
//! 本模块位于 `yeban-midi` crate，只依赖 `yeban-model` 与 `midly`，零 GUI 依赖、
//! 零音频/系统库依赖。纯映射函数 [`plan_midi_import`] 产出结构化的 [`MidiImportPlan`]，
//! 不触碰撤销栈、不直接修改工程、不碰文件系统。
//! `yeban-app` 与 `yeban-mcp` 可直接复用本模块的纯映射。
//!
//! ## 数据流（纯映射层）
//!
//! ```text
//! bytes ──▶ yeban_midi::midi::parse_smf（既有解析器）
//!       ──▶ ParsedNote ⇒ MidiNote（PPQ 换算到模型的 960；绝对 tick 口径与铅笔一致）
//!       ──▶ Op::AddNote{track_id, clip_id, note} ＋ 导体元数据 Op
//!       ──▶ MidiImportPlan { ops, losses, lyrics, .. }
//! ```
//!
//! ## 口径（⭐ 本导入**只**消费什么、**不**消费什么）
//!
//! 1. **音符映射消费** [`ParsedMidi::notes`] / [`ParsedMidi::lyrics`] / [`ParsedMidi::ppq`]；
//!    ⭐ `#342` 起**另有四类元数据有落点**：`parsed.tempos`（只取
//!    `microseconds_per_quarter` / `numerator` 字段）· `parsed.track_names` ·
//!    `parsed.markers` · `parsed.end_tick`。其余读数仍只读 `format` / `tracks` /
//!    `aux_events`（⛔ 不参与映射）。
//! 2. **tempo / 拍号只在"目标工程没有内容"时采纳**：`project.bpm` 与
//!    `project.time_signature` 都是**单标量**，而且前者是**全局**的 —— 改它会重新
//!    计时整首曲子。所以两个条件同时成立才写：① 文件只有**唯一条生效值且在 tick 0**；
//!    ② [`import_target_is_empty`]（无段落 / 无 MIDI 音符 / 无音频片段 / 无自动化点；
//!    落点片段与摆放**不算内容**）。否则整类报 `dropped`，原因里**点名**文件的值与
//!    tick，并说明"工程已有内容，因此没有覆盖全局值"。map（多条、或首条不在 0）同样
//!    整类报 `dropped` 并列出每个值。
//!    音符的**绝对 tick** 仍按 `ppq` 换算后**原样**落到工程上，⛔ 不做与速度有关的换算。
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
//!    [`crate::midi::parse_smf`] 及其错误变体的事（⭐ 指路，⛔ 不手抄）。
//! 7. **四类元数据有落点**（⭐ `#342`，⛔ 全是"无歧义才落"）：
//!    - `FF 06` marker → [`yeban_model::SectionV3`]：相邻 marker 之间一个 tick 区间，
//!      最后一个到 [`ParsedMidi::end_tick`]。同一 tick 的后一条顶掉前一条（被顶掉的
//!      逐条计入 `dropped`）；末尾与 `end_tick` 重合（**零长度**）落不下去、也计入
//!      `dropped`；非 UTF-8 文本仍是**边界**（前一段在它那里结束），只丢名字。
//!      ⚠ **段落是追加的**：工程里**已有**的段落一个字不动，本导入**不**清空、也
//!      **不**检查它们 —— 不重叠只对**本次导入产生的那一组**成立（模型
//!      `SectionV3::validate` 与 `YebanProjectV1::validate` 都不查重叠）。
//!    - `FF 03` 轨名 → 目标音轨的 `TrackV3::name`：去重后**恰一个**非空、严格 UTF-8
//!      名字才落（多个 `MTrk` 同名是同一个名字）；多个不同名字 / 空名 / 非 UTF-8
//!      一律不落（⛔ 挑一个）。
//!    - `FF 51` tempo → `project.bpm`（单标量）：**唯一条生效 tempo 且 tick 0** 且目标
//!      工程没有内容时才落；否则一字不动并报出"会采纳的值"。
//!    - `FF 58` 拍号 → `project.time_signature`（单标量）：同一条内容门槛。
//! 8. **其余 17 类只报读数、⛔ 不新增支持**（`#318`）：[`ParsedMidi::aux_events`] 把
//!    24 类具名事件逐一计数，本文件逐类给出 `read` / `dropped`；无落点的类
//!    `dropped == read`。报告字段：`format=` / `file-tracks=` /
//!    `track-partitions-dropped=` / `channels=`（上下文，总是打印）＋ 每类
//!    `<kind>-read=N <kind>-dropped=N`（仅 `read>0`）＋ 有损失时的
//!    `<kind>-reason="<精确原因，含具体值>"` ＋ `note-channel-*` /
//!    `note-off-velocity-*`（按音符计）。
//!    ⚠ **"值一样"不等于"没丢"**：判据是"有没有被写进工程"。`dropped` 是**事件
//!    条数**里没有落进工程的条数；同一 tick 被顶掉的事件只在**从未生效**时算落地
//!    （见第 7 条的 marker 规则）。

use std::collections::{BTreeMap, BTreeSet};

use crate::midi::{MidiAuxEventKind, MidiFormat, ParsedMarker, ParsedMidi, ParsedTrackName};
use yeban_model::project::{MAX_BPM, MIN_BPM};
use yeban_model::{
    AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, CurveType, EntityId, KeySignature, MidiNote, Op, RoutingEdge,
    RoutingKind, SectionV3, TimeSignature, TrackKind, TrackV3, YebanProjectV1,
};

/// 模型的每四分音符 tick 数（960 PPQ）。导入时把文件的 PPQ 换算到它。
pub const MODEL_PPQ: u64 = 960;

/// ⭐ `#342` / `#350`：今天**有落点**的七类 aux 事件（弯音自 ADR-0014 D2 起有落点；
/// 控制器自 ADR-0015 D2 起 CC 7 / CC 10 有落点）。
///
/// 其余 17 类由 [`MidiImportLosses::is_total_loss`] 钉住"仍全损"。
pub const STORABLE_KINDS: [MidiAuxEventKind; 7] = [
    MidiAuxEventKind::Tempo,
    MidiAuxEventKind::TimeSignature,
    MidiAuxEventKind::TrackName,
    MidiAuxEventKind::Marker,
    MidiAuxEventKind::KeySignature,
    MidiAuxEventKind::PitchBend,
    MidiAuxEventKind::ControlChange,
];

/// 今天有落点的七类 aux 事件的稳定键名（与 [`STORABLE_KINDS`] 对应）。
pub const STORABLE_KIND_KEYS: &[&str] = &[
    "tempo",
    "time-signature",
    "track-name",
    "marker",
    "key-signature",
    "pitch-bend",
    "control-change",
];

/// ⭐ ADR-0015 D2：通道音量控制器号 → `AutomationTarget::TrackVolume`。
pub const CC_CHANNEL_VOLUME: u8 = 7;

/// ⭐ ADR-0015 D2：声相控制器号 → `AutomationTarget::TrackPan`。
pub const CC_PAN: u8 = 10;

/// ⭐ ADR-0015 D2：CC 7（GM 音量）⇒ 分贝。
///
/// GM 标准的音量增益是 `(v / 127)²` ⇒ 分贝 = `20 × log10(v/127)²` = `40 × log10(v/127)`。
/// `v = 0` 单独走 `-60.0`（`log10(0)` 是 `-inf`，落不进 `f32` 的有限值判据）；
/// `v = 127` ⇒ `0.0` dB；`v = 100`（MIDI 默认音量）⇒ 约 `-4.15` dB。
/// 结果钳在音量泳道的固有值域 `[-60.0, 12.0]` 内。
#[must_use]
pub fn cc7_to_decibels(value: u8) -> f32 {
    if value == 0 {
        return -60.0;
    }
    (40.0 * libm::log10f(f32::from(value) / 127.0)).clamp(-60.0, 12.0)
}

/// ⭐ ADR-0015 D2：CC 10（声相）⇒ 双极 `-1.0..=1.0`。
///
/// 中点是 64（极左 `0` / 居中 `64` / 极右 `127`）。左右**各用自身的段宽**做分母
/// （左 64 格、右 63 格）⇒ 两端恰好落在 `-1.0` 与 `+1.0`，中点不偏。
#[must_use]
pub fn cc10_to_pan(value: u8) -> f32 {
    let v = f32::from(value);
    if value <= 64 {
        (v - 64.0) / 64.0
    } else {
        (v - 64.0) / 63.0
    }
}

/// 导入选项。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MidiImportOptions {
    /// 目标音轨；`None` = 第一条**非 master** 的音轨。
    pub track_id: Option<EntityId>,
    /// 目标片段；`None` = 目标音轨的**第一个**摆放（`placement`）。
    pub clip_id: Option<EntityId>,
    /// 是否拆分多轨（若为 true 且为多轨文件，含有音符的后续轨道将自动创建新音轨/片段/摆放/路由；若为 false，所有音符塞入目标片段）。
    pub split_tracks: bool,
}

impl Default for MidiImportOptions {
    fn default() -> Self {
        Self {
            track_id: None,
            clip_id: None,
            split_tracks: true,
        }
    }
}

/// ⭐ `#317`：一次导入的歌词读数（**唯一**计算点在 [`plan_midi_import`]/[`attach_lyrics`])。
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

    /// ⭐ `#320`：可见行只列**非零的歌词丢失桶**（`dropped` / `non_utf8` / `empty`）。
    ///
    /// 全零 ⇒ 空串（可见行不凭空多出"歌词…0"）。`read` / `attached` 是正向上下文，
    /// 留在 [`core::fmt::Display`] 的完整五字段里（CLI 行与 GUI `lyrics_detail`）。
    ///
    /// 只有 `dropped` 时用最清楚的一版（`歌词未归属 1`）；一旦 `non_utf8` / `empty`
    /// 也非零，改用斜杠紧凑版（`歌词未归属1/非UTF8 1/空2`）—— 实测后者在不带
    /// `通道/力度` 时 `spare=14`，带则溢出 264px（见 [`MidiImportLosses::loss_summary`]）。
    #[must_use]
    pub fn loss_fragment(self) -> String {
        if self.dropped == 0 && self.non_utf8 == 0 && self.empty == 0 {
            return String::new();
        }
        if self.non_utf8 == 0 && self.empty == 0 {
            return format!("歌词未归属 {}", self.dropped);
        }
        let mut parts: Vec<String> = Vec::new();
        if self.dropped > 0 {
            parts.push(format!("未归属{}", self.dropped));
        }
        if self.non_utf8 > 0 {
            parts.push(format!("非UTF8 {}", self.non_utf8));
        }
        if self.empty > 0 {
            parts.push(format!("空{}", self.empty));
        }
        format!("歌词{}", parts.join("/"))
    }
}

impl core::fmt::Display for MidiLyricCounts {
    /// ⭐ `#320`：`lyrics= attached= dropped= non-utf8= empty=` 五字段的唯一格式化。
    ///
    /// 逐字与 `#317`／`#318` 起的 CLI 行相同（`imported-midi:` 行里的这一段）。
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            formatter,
            "lyrics={} attached={} dropped={} non-utf8={} empty={}",
            self.read, self.attached, self.dropped, self.non_utf8, self.empty
        )
    }
}

/// ⭐ `#318`：一类丢失的**两列读数**。
///
/// `read` 是文件里读到的条数（判据另按字节独立数一遍做控制臂）；
/// `dropped` 是导入路径**没有**落进工程的条数。
///
/// ⭐ `#342` 起有落点的**七类**（tempo / 拍号 / 轨名 / marker / 调号 / 弯音 / 控制器，
/// 见 [`STORABLE_KINDS`]）**可能** `dropped < read`；其余 17 类与音符通道 / 释放力度
/// 仍是 `dropped == read`。
///
/// ⚠ **"值一样"不等于"没丢"**：判据是"**有没有被写进工程**"，⛔ 不是"数值一不一样"。
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

    /// 读到多少就丢多少（某一类**完全没有**落点时的读数）。
    ///
    /// ⚠ 这是一条**会过期**的判据：某一类从"无落点"变成"有落点"时，它必须变成
    /// `false`，相应断言要**改掉**；⛔ 不许把判据删掉 —— 它过期时应当自己说出来。
    #[must_use]
    pub const fn is_total_loss(self) -> bool {
        self.dropped == self.read
    }
}

/// ⭐ `#318`：一次导入的**其余丢失读数**（音符 / 歌词以外）。
///
/// ## 每一类今天**有没有**落点（`#342` 重新核对；⛔ 不是"值一样就算没丢"）
///
/// - `tempo` / 拍号：落点是工程的 `bpm` / `time_signature`（都是**单标量**）⇒
///   **只有**"唯一条生效值在 tick 0"能落；map 整类 `dropped` 并在
///   [`Self::aux_reasons`] 里列出每个值。
/// - `FF 03` 轨名：落点是目标音轨的 `TrackV3::name`；去重后恰一个非空、严格
///   UTF-8 名字才落。
/// - `FF 06` marker：落点是 `YebanProjectV1::sections` 的 `SectionV3` 区间；
///   零长度 / 非 UTF-8 的逐条 `dropped`。
/// - `E0` 弯音：落点是**唯一发声音符**的 `MidiNote::pitch_bend_curve`
///   （ADR-0014 D2，与歌词同一条"唯一才归属"原则）；0 颗或多颗发声的条 `dropped`。
/// - `FF 59` 调号 / CC / 触后 / Program Change / SysEx / Escape / 其余 meta：
///   模型或导入映射没有落点 ⇒ **全丢**（原因见 [`Self::aux_reasons`]）。
/// - 轨分区：全部音符进**同一个**音轨与同一个片段 ⇒ 文件 N 条 `MTrk` 只剩 1 个落点，
///   `N-1` 个分区丢失。
/// - 音符通道：[`MidiNote`] 的 channel 字段保存每颗音符的通道值（`#348` [MODEL-AST-005]）。
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
    /// 24 类 aux 事件，逐类 read/dropped。
    ///
    /// ⭐ `#342` / ADR-0014 / ADR-0015：有落点的**七类**（tempo / 拍号 / 轨名 / marker /
    /// 调号 / 弯音 / 控制器）**可能** `dropped < read`（部分内容真的进了工程）；
    /// 其余 17 类仍是全损。
    pub aux_events: BTreeMap<MidiAuxEventKind, MidiLossCount>,
    /// ⭐ `#342`：`dropped > 0` 的类别的**精确原因**（键 = 与报告字段同一个稳定键
    /// [`MidiAuxEventKind::key`]，⛔ 不翻译；值是含**具体数值**的一句话）。
    ///
    /// 这是**结构化数据**（`String`），不是排版：MCP 可以把键与值原样放进响应；
    /// GUI/CLI 只在 `loss_detail` 里打印它（**不进** 264px 的可见摘要行）。
    pub aux_reasons: BTreeMap<MidiAuxEventKind, String>,
}

impl MidiImportLosses {
    /// 自检：每条 `dropped <= read`；`track_partitions_dropped <= file_tracks - 1`；
    /// 原因表只给 `dropped > 0` 的类别（⛔ 不许给"没丢"的类别编故事）。
    #[must_use]
    pub fn is_consistent(&self) -> bool {
        self.file_tracks >= 1
            && self.track_partitions_dropped <= self.file_tracks.saturating_sub(1)
            && self.note_channel_values.is_consistent()
            && self.note_off_velocity_values.is_consistent()
            && self.aux_events.values().all(|count| count.is_consistent())
            && self.aux_reasons.iter().all(|(kind, reason)| {
                !reason.is_empty()
                    && self
                        .aux_events
                        .get(kind)
                        .is_some_and(|count| count.dropped > 0)
            })
    }

    /// ⭐ `#342`：**仍然没有落点**的类别一律全损 ⇒ 这条判据为真。
    ///
    /// 四类有落点的事件（[`STORABLE_KINDS`]）以及已有落点的音符通道（当
    /// [`channel_is_stored_on_note()`] 为真）**不参与**：它们可能部分或完全落地。
    /// 释放力度今天仍是全损。
    ///
    /// ⚠ 这仍是一条**会过期**的判据：若某类从"有落点"退回"全损"、或反过来，
    /// 它应该红 —— ⛔ 不是把它删掉。
    #[must_use]
    pub fn is_total_loss(&self) -> bool {
        (channel_is_stored_on_note() || self.note_channel_values.is_total_loss())
            && self.note_off_velocity_values.is_total_loss()
            && self
                .aux_events
                .iter()
                .filter(|(kind, _)| !STORABLE_KINDS.contains(kind))
                .all(|(_, count)| count.is_total_loss())
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

    /// 是否没有发生任何损失（全部事件与通道/力度都完整保留）。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.loss_total() == 0
    }

    /// 损失的事件总条数（含轨分区合并、通道值丢弃、释放力度丢弃、未落地 aux 事件）。
    #[must_use]
    pub fn loss_total(&self) -> usize {
        self.track_partitions_dropped
            + self.note_channel_values.dropped
            + self.note_off_velocity_values.dropped
            + self.dropped_events()
    }

    /// 损失读数的 JSON 字符串表示（供 MCP / 诊断使用）。
    #[must_use]
    pub fn to_json(&self) -> String {
        let mut aux = Vec::new();
        for (kind, count) in &self.aux_events {
            let reason = self.aux_reasons.get(kind).map_or("", String::as_str);
            let safe_reason = reason.replace('\\', "\\\\").replace('"', "\\\"");
            aux.push(format!(
                r#""{}":{{"read":{},"dropped":{},"reason":"{}"}}"#,
                kind.key(),
                count.read,
                count.dropped,
                safe_reason
            ));
        }
        format!(
            r#"{{"format":{},"file_tracks":{},"track_partitions_dropped":{},"distinct_channels":{},"note_channel_values":{{"read":{},"dropped":{}}},"note_off_velocity_values":{{"read":{},"dropped":{}}},"aux_events":{{{}}},"loss_total":{}}}"#,
            self.format.number(),
            self.file_tracks,
            self.track_partitions_dropped,
            self.distinct_channels,
            self.note_channel_values.read,
            self.note_channel_values.dropped,
            self.note_off_velocity_values.read,
            self.note_off_velocity_values.dropped,
            aux.join(","),
            self.loss_total()
        )
    }

    /// 24 类 aux 事件的逐类字段串（`<kind>-read=N <kind>-dropped=N …`）。
    ///
    /// ⭐ `#319`：这是**唯一**的逐类格式化处 —— `MidiImportReport::summary`（CLI）
    /// 与 GUI 的 `loss_detail="…"` 字段**共用**它 ⇒ ⛔ 不是第二份格式化。
    ///
    /// 只有 `read > 0` 的类才打印（`#318` 起的 CLI 口径）；每类前带一个空格 ⇒
    /// 空表返回空串，非空表可直接拼在上一字段之后。字段里的 `=` / 空格在 GUI 回执里
    /// 整体落在**引号内**，不参与回执语法（同 `clipping_note` 先例）。
    ///
    /// ⭐ `#342`：`dropped > 0` 的类别追加 `<kind>-reason=<精确原因>`（含具体数值）。
    /// 原因原样存在于 [`Self::aux_reasons`]（结构化数据）；打印前经 [`receipt_safe`]
    /// 处理 —— 回执的 `loss_detail="…"` 是**引号定界**且解析器不认转义 ⇒
    /// ⛔ 原因里不能再出现 `"`（连**包住原因的引号**也不行：那会提前闭合
    /// `loss_detail`），换行/制表也换成空格。
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
            if let Some(reason) = self.aux_reasons.get(kind) {
                out.push_str(&format!(" {}-reason={}", kind.key(), receipt_safe(reason)));
            }
        }
        out
    }

    /// 明细字段的完整值（GUI 回执的 `loss_detail="…"`）：4 项上下文 ＋ 按音符 4 项
    /// ＋ [`Self::aux_fields`] 的 24 类逐类读数。
    ///
    /// ⭐ `#319`：CLI 的 `imported-midi:` 行尾部也**直接拼这一串**
    /// （`MidiImportReport::summary`）⇒ 两个出口的明细来自**同一个格式化函数**。
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
    /// `通道` / `力度` = 按音符计的 `dropped`；`歌词…` = [`MidiLyricCounts`] 的
    /// **非零丢失桶**（`dropped` / `non_utf8` / `empty`；全零 ⇒ 这一截省略）。
    /// 无 aux 丢失时省略"0 类 / 0 条"那一截，只留按音符与歌词的读数（仍然如实说）。
    ///
    /// ⭐ `#320` 版面（**实测**，⛔ 推算）：`#319` 原文右沿 1881 / 行右沿 1912
    /// （余 31px）；原文＋歌词 ⇒ `spare=1`（1px，过不了 `#319` 自己的 **3px** 门）。
    /// 压缩后可放：最紧 4px、推荐 7–11px、换前缀 17px。本行取
    /// `⚠ 导入丢失：24 类/29 条；歌词未归属 1；通道/力度5/5`（`spare=7`）——
    /// 保既有六项语义（导入丢失 / 类数 / 事件数 / 歌词未归属 / 通道 / 释放力度），
    /// 只压缩空格与标签词，⛔ 不丢桶（丢通道/力度那一版已否）。`3px` 门是硬约束。
    #[must_use]
    pub fn loss_summary(&self, lyrics: MidiLyricCounts) -> String {
        let classes = self
            .aux_events
            .values()
            .filter(|count| count.dropped > 0)
            .count();
        let events = self.dropped_events();
        let lyric_loss = lyrics.loss_fragment();
        let channels = self.note_channel_values.dropped;
        let velocities = self.note_off_velocity_values.dropped;
        if lyric_loss.is_empty() {
            // ⚠ 无歌词丢失 ⇒ **逐字保持 `#319` 的旧文本**（不凭空多出"歌词…0"）。
            if events == 0 {
                format!("⚠ 导入丢失：通道值 {channels}、释放力度 {velocities}")
            } else {
                format!(
                    "⚠ 导入丢失：{classes} 类 / {events} 条；通道值 {channels}、释放力度 {velocities}"
                )
            }
        } else if lyrics.non_utf8 > 0 || lyrics.empty > 0 {
            // ⚠ 非 UTF-8 / 空桶也非零 ⇒ 歌词丢失优先占可见位：这一版已是 264px
            // 内的上限（`spare=14`），再加 `通道/力度` 会溢出 ⇒ 后两项留在
            // `loss_detail="…"`（⛔ 不是从数据通道里删除）。
            if events == 0 {
                format!("⚠ 导入丢失：{lyric_loss}")
            } else {
                format!("⚠ 导入丢失：{classes} 类/{events} 条；{lyric_loss}")
            }
        } else if events == 0 {
            format!("⚠ 导入丢失：{lyric_loss}；通道/力度{channels}/{velocities}")
        } else {
            format!(
                "⚠ 导入丢失：{classes} 类/{events} 条；{lyric_loss}；通道/力度{channels}/{velocities}"
            )
        }
    }
}

/// 导入计划（**纯函数产物**：提交之前的一切）。
#[derive(Clone, Debug, PartialEq)]
pub struct MidiImportPlan {
    /// 待提交的操作（每个音符一个 `Op::AddNote` ＋ 元数据操作）。
    pub ops: Vec<Op>,
    /// 音符条数。
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

/// 回执里**安全**的一串。
///
/// GUI 的 `loss_detail="…"` 是**引号定界**，而回执解析器（`host::module_receipt`）
/// 不认转义 ⇒ 原因里的 `"` 与换行若原样进串会把回执截断。数据本身仍原样留在
/// [`MidiImportLosses::aux_reasons`]；这里只改**打印**形态。
fn receipt_safe(text: &str) -> String {
    text.chars()
        .map(|ch| match ch {
            '"' => '\'',
            '\n' | '\r' | '\t' => ' ',
            other => other,
        })
        .collect()
}

/// `FF 51` 的 `mpqn` ⇒ BPM。`0` 没有定义（除零）⇒ `None`。
fn bpm_from_microseconds(microseconds_per_quarter: u32) -> Option<f64> {
    if microseconds_per_quarter == 0 {
        None
    } else {
        Some(60_000_000.0 / f64::from(microseconds_per_quarter))
    }
}

/// 单个标量（或同类标量序列）的采纳结果：要追加的 ops + `dropped` 条数 + 原因。
pub struct ScalarAdoption {
    /// 需要写进工程的操作（`None` = 现值已经等于文件值，或落不下去）。
    pub op: Option<Op>,
    /// 多点操作（例如 SetTempoPoint / SetTimeSignaturePoint，或当有多条操作需要发射时）。
    pub ops: Vec<Op>,
    /// 没有落进工程的**事件条数**（⛔ 不是"不同值个数"）。
    pub dropped: usize,
    /// `dropped > 0` 时的精确原因（含具体值；⛔ 空串）。
    pub reason: Option<String>,
}

impl ScalarAdoption {
    pub fn all_ops(&mut self) -> Vec<Op> {
        let mut list = Vec::new();
        if let Some(op) = self.op.take() {
            list.push(op);
        }
        list.append(&mut self.ops);
        list
    }
}

/// 导入目标是否**没有内容**（`#342` 修订：决定 bpm / 拍号能不能被采纳）。
///
/// ## 精确口径
///
/// "没有内容" = **没有任何会因 bpm/拍号改变而被重新计时或重新划小节的东西**：
///
/// 1. `project.sections` 为空（段落是 tick 区间）；
/// 2. `clip_pool` 里**没有**带音符的 MIDI 片段；
/// 3. `clip_pool` 里**没有**音频片段（`ClipContent::Audio`）；
/// 4. 任何音轨的自动化泳道**没有点**（自动化点是 tick 坐标）。
///
/// ⚠ **落点片段 / 摆放不算内容**：`destination()` 要求导入有一个落点（CLI 与 GUI
/// 都走 `MidiImportOptions::default()`），空 MIDI 片段 + 摆放只是"槽位"。
/// ⇒ 字面上的"任何音轨上都没有摆放"在本导入路径里**不可达**；把零摆放当条件会
/// 让"采纳"分支永远进不去。
#[must_use]
pub fn import_target_is_empty(project: &YebanProjectV1) -> bool {
    project.sections.is_empty()
        && !project
            .clip_pool
            .values()
            .any(|entry| match &entry.content {
                ClipContent::Midi { notes } => !notes.is_empty(),
                ClipContent::Audio { .. } => true,
            })
        && project.tracks.values().all(|track| {
            track
                .automation_lanes
                .values()
                .all(|lane| lane.points.is_empty())
        })
}

/// 测试夹具与基线：一个**没有内容**的工程（演示工程去掉段落 / 音符 / 音频片段 /
/// 自动化点，保留音轨、空 MIDI 片段与落点 ⇒ 导入有落点且 bpm/拍号可被采纳）。
#[must_use]
pub fn content_free_project() -> YebanProjectV1 {
    let mut project = yeban_model::samples::demo_project();
    project.sections.clear();
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            notes.clear();
        }
    }
    let audio_clips: BTreeSet<EntityId> = project
        .clip_pool
        .iter()
        .filter(|(_, entry)| matches!(&entry.content, ClipContent::Audio { .. }))
        .map(|(id, _)| *id)
        .collect();
    for track in project.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| !audio_clips.contains(&placement.clip_id));
        for lane in track.automation_lanes.values_mut() {
            lane.points.clear();
        }
    }
    project.clip_pool.retain(|id, _| !audio_clips.contains(id));
    project.validate().expect("content-free 夹具必须整体合法");
    debug_assert!(import_target_is_empty(&project));
    project
}

/// 音符实体是否存储 MIDI 通道号。
///
/// 契约说明：`#348` [MODEL-AST-005] [ARCH-EXT-002] 为 `MidiNote` 增加了 `channel: Option<u8>` 字段，导入时保留音符通道。
#[inline]
const fn channel_is_stored_on_note() -> bool {
    true
}

/// 派生新实体 ID。
#[inline]
fn derive_note_id() -> EntityId {
    EntityId::new()
}

/// `FF 51` tempo 采纳计算（同 [`adopt_tempo`]）。
#[inline]
fn tempo_adoption_op(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    adopt_tempo(parsed, project, target_empty)
}

/// `FF 51` tempo → `project.bpm` 与 `project.tempo_map`。
///
/// tick 0 的生效值写进 `project.bpm`（若与现值不同），tick > 0 的生效值写进 `project.tempo_map`。
/// 同一 tick 的多条按文件顺序后出现者胜出（MIDI 的生效语义）⇒ 生效 map 用 `BTreeMap`。
///
/// ⚠ `#342` 修订：只有目标工程没有内容（[`import_target_is_empty`]）时才写工程；
/// 否则 `bpm` / `tempo_map` 一字不动，并把所有值作为**响亮的损失**报出（含值与 tick）。
pub fn adopt_tempo(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    let read = parsed.aux_events.count(MidiAuxEventKind::Tempo);
    debug_assert_eq!(
        read,
        parsed
            .tempos
            .iter()
            .filter(|record| record.microseconds_per_quarter.is_some())
            .count(),
        "每条 `FF 51` 必须恰产生一条带 mpqn 的记录"
    );
    if read == 0 {
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: 0,
            reason: None,
        };
    }
    let mut effective: BTreeMap<u64, u32> = BTreeMap::new();
    for record in &parsed.tempos {
        if let Some(microseconds_per_quarter) = record.microseconds_per_quarter {
            let tick = scale_ticks(record.tick, parsed.ppq);
            effective.insert(tick, microseconds_per_quarter);
        }
    }

    let mut values = Vec::new();
    if effective.keys().next().is_some_and(|tick| *tick > 0) {
        values.push("120.00 BPM @0 (SMF default: no tempo event)".to_owned());
    }
    for (tick, microseconds_per_quarter) in &effective {
        values.push(match bpm_from_microseconds(*microseconds_per_quarter) {
            Some(bpm) => format!("{bpm:.2} BPM @{tick}"),
            None => format!("undefined (mpqn=0) @{tick}"),
        });
    }

    for (tick, mpqn) in &effective {
        if bpm_from_microseconds(*mpqn).is_none() {
            let reason = if effective.len() == 1 && *tick == 0 {
                "tempo-undefined: the file's single tempo event has \
                 microseconds-per-quarter = 0 (no BPM is defined); project.bpm untouched"
                    .to_owned()
            } else {
                format!(
                    "tempo-undefined: a tempo event @{tick} has \
                     microseconds-per-quarter = 0 (no BPM is defined); project tempo untouched"
                )
            };
            return ScalarAdoption {
                op: None,
                ops: Vec::new(),
                dropped: read,
                reason: Some(reason),
            };
        }
    }

    for (tick, mpqn) in &effective {
        let bpm = bpm_from_microseconds(*mpqn).unwrap();
        if !(MIN_BPM..=MAX_BPM).contains(&bpm) {
            let reason = if effective.len() == 1 && *tick == 0 {
                format!(
                    "tempo-out-of-model-range: the file's single tempo is {bpm:.2} BPM, \
                     outside the model's {MIN_BPM:.0}..={MAX_BPM:.0} BPM; project.bpm untouched"
                )
            } else {
                format!(
                    "tempo-out-of-model-range: tempo {bpm:.2} BPM @{tick} is outside \
                     the model's {MIN_BPM:.0}..={MAX_BPM:.0} BPM; project tempo untouched"
                )
            };
            return ScalarAdoption {
                op: None,
                ops: Vec::new(),
                dropped: read,
                reason: Some(reason),
            };
        }
    }

    if !target_empty {
        let reason = if effective.len() == 1 && effective.keys().next() == Some(&0) {
            let bpm = bpm_from_microseconds(*effective.values().next().unwrap()).unwrap();
            format!(
                "tempo-not-adopted-non-empty-project: the file's single tempo \
                 ({bpm:.2} BPM @0) was NOT written to project.bpm — the project \
                 already has content (sections / MIDI notes / audio clips / \
                 automation points) and rewriting the global tempo would re-time it"
            )
        } else {
            format!(
                "tempo-not-adopted-non-empty-project: the file's tempo map \
                 ({}) was NOT written to project — the project already has \
                 content (sections / MIDI notes / audio clips / automation \
                 points) and rewriting the tempo would re-time it",
                values.join("; ")
            )
        };
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: read,
            reason: Some(reason),
        };
    }

    let mut op = None;
    let mut ops = Vec::new();

    if effective.keys().next().is_some_and(|tick| *tick > 0)
        && (project.bpm - 120.0).abs() > f64::EPSILON
    {
        op = Some(Op::SetTempo {
            old_bpm: project.bpm,
            new_bpm: 120.0,
        });
    }

    for (tick, mpqn) in &effective {
        let bpm = bpm_from_microseconds(*mpqn).expect("already validated");
        if *tick == 0 {
            if (project.bpm - bpm).abs() > f64::EPSILON {
                op = Some(Op::SetTempo {
                    old_bpm: project.bpm,
                    new_bpm: bpm,
                });
            }
        } else {
            ops.push(Op::SetTempoPoint {
                tick: *tick,
                old_bpm: None,
                new_bpm: Some(bpm),
            });
        }
    }

    ScalarAdoption {
        op,
        ops,
        dropped: 0,
        reason: None,
    }
}

/// 拍号的文本形态（分母是 SMF 的以 2 为底的幂）。
fn meter_label(numerator: u8, denominator_pow2: u8) -> String {
    if denominator_pow2 <= 5 {
        format!("{numerator}/{}", 1u8 << denominator_pow2)
    } else {
        format!("{numerator}/(2^{denominator_pow2})")
    }
}

/// `FF 58` 拍号采纳计算（同 [`adopt_time_signature`]）。
#[inline]
fn time_signature_adoption_op(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    adopt_time_signature(parsed, project, target_empty)
}

/// `FF 58` 拍号 → `project.time_signature` 与 `project.time_signature_map`。
///
/// tick 0 的生效值写进 `project.time_signature`（若与现值不同），tick > 0 的生效值写进 `project.time_signature_map`。
/// 同一 tick 的多条按文件顺序后出现者胜出（MIDI 的生效语义）⇒ 生效 map 用 `BTreeMap`。
///
/// ⚠ `#342` 修订：只有目标工程没有内容（[`import_target_is_empty`]）时才写工程；
/// 否则 `time_signature` / `time_signature_map` 一字不动，并把所有值作为**响亮的损失**报出。
pub fn adopt_time_signature(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    let read = parsed.aux_events.count(MidiAuxEventKind::TimeSignature);
    debug_assert_eq!(
        read,
        parsed
            .tempos
            .iter()
            .filter(|record| record.numerator.is_some() && record.denominator_pow2.is_some())
            .count(),
        "每条 `FF 58` 必须恰产生一条带分子的记录"
    );
    if read == 0 {
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: 0,
            reason: None,
        };
    }
    let mut effective: BTreeMap<u64, (u8, u8)> = BTreeMap::new();
    for record in &parsed.tempos {
        if let (Some(numerator), Some(denominator_pow2)) =
            (record.numerator, record.denominator_pow2)
        {
            let tick = scale_ticks(record.tick, parsed.ppq);
            effective.insert(tick, (numerator, denominator_pow2));
        }
    }

    let mut values = Vec::new();
    if effective.keys().next().is_some_and(|tick| *tick > 0) {
        values.push("4/4 @0 (SMF default: no time-signature event)".to_owned());
    }
    for (tick, &(numerator, denominator_pow2)) in &effective {
        values.push(format!(
            "{} @{tick}",
            meter_label(numerator, denominator_pow2)
        ));
    }

    for (tick, &(numerator, denominator_pow2)) in &effective {
        let denominator = if denominator_pow2 <= 5 {
            1u8 << denominator_pow2
        } else {
            0
        };
        let candidate = TimeSignature {
            numerator,
            denominator,
        };
        if candidate.validate().is_err() {
            let reason = if effective.len() == 1 && *tick == 0 {
                format!(
                    "time-signature-out-of-model-range: the file's single meter is {}, outside \
                     the model's numerator 1..=32 / denominator 1,2,4,8,16,32; \
                     project.time_signature untouched",
                    meter_label(numerator, denominator_pow2)
                )
            } else {
                format!(
                    "time-signature-out-of-model-range: meter {} @{tick} is outside \
                     the model's numerator 1..=32 / denominator 1,2,4,8,16,32; \
                     project time_signature untouched",
                    meter_label(numerator, denominator_pow2)
                )
            };
            return ScalarAdoption {
                op: None,
                ops: Vec::new(),
                dropped: read,
                reason: Some(reason),
            };
        }
    }

    if !target_empty {
        let reason = if effective.len() == 1 && effective.keys().next() == Some(&0) {
            let (numerator, denominator_pow2) = *effective.values().next().unwrap();
            format!(
                "time-signature-not-adopted-non-empty-project: the file's single \
                 meter ({} @0) was NOT written to project.time_signature — the \
                 project already has content (sections / MIDI notes / audio clips / \
                 automation points) and rewriting the global meter would re-bar it",
                meter_label(numerator, denominator_pow2)
            )
        } else {
            format!(
                "time-signature-not-adopted-non-empty-project: the file's meter map \
                 ({}) was NOT written to project — the project already has \
                 content (sections / MIDI notes / audio clips / automation \
                 points) and rewriting the meter would re-bar it",
                values.join("; ")
            )
        };
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: read,
            reason: Some(reason),
        };
    }

    let mut op = None;
    let mut ops = Vec::new();

    if effective.keys().next().is_some_and(|tick| *tick > 0) {
        let smf_default = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        if project.time_signature != smf_default {
            op = Some(Op::SetTimeSignature {
                old_time_signature: project.time_signature,
                new_time_signature: smf_default,
            });
        }
    }

    for (tick, &(numerator, denominator_pow2)) in &effective {
        let candidate = TimeSignature {
            numerator,
            denominator: 1u8 << denominator_pow2,
        };
        if *tick == 0 {
            if project.time_signature != candidate {
                op = Some(Op::SetTimeSignature {
                    old_time_signature: project.time_signature,
                    new_time_signature: candidate,
                });
            }
        } else {
            ops.push(Op::SetTimeSignaturePoint {
                tick: *tick,
                old_time_signature: None,
                new_time_signature: Some(candidate),
            });
        }
    }

    ScalarAdoption {
        op,
        ops,
        dropped: 0,
        reason: None,
    }
}

/// `FF 59` 调号采纳计算（同 [`adopt_key_signature`]）。
#[inline]
fn key_signature_adoption_op(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    adopt_key_signature(parsed, project, target_empty)
}

/// `FF 59` 调号 → `project.key_signature`（**单标量**）。
///
/// 只有"唯一条生效调号且在 tick 0"能落。
/// 门槛：与 tempo / 拍号同一条内容门槛（[`import_target_is_empty`]）——
/// 目标工程没有内容（无段落 / 无音符 / 无音频片段 / 无自动化点）时才采纳；
/// 否则不修改工程，作为丢失原因报出。
pub fn adopt_key_signature(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    target_empty: bool,
) -> ScalarAdoption {
    let read = parsed.aux_events.count(MidiAuxEventKind::KeySignature);
    let mut effective: BTreeMap<u64, (i8, bool)> = BTreeMap::new();
    for record in &parsed.key_signatures {
        effective.insert(record.tick, (record.sharps_flats, record.is_minor));
    }
    if read == 0 {
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: 0,
            reason: None,
        };
    }
    if let (1, Some((&0, &(sharps_flats, is_minor)))) = (effective.len(), effective.iter().next()) {
        let candidate = KeySignature {
            sharps_flats,
            is_minor,
        };
        if candidate.validate().is_ok() {
            if !target_empty {
                return ScalarAdoption {
                    op: None,
                    ops: Vec::new(),
                    dropped: read,
                    reason: Some(format!(
                        "key-signature-not-adopted-non-empty-project: the file's single key signature ({sharps_flats} sf, minor={is_minor} @0) was NOT written to project.key_signature — the project already has content (sections / MIDI notes / audio clips / automation points)",
                    )),
                };
            }
            let op = if project.key_signature == Some(candidate) {
                None
            } else {
                Some(Op::SetKeySignature {
                    old_key_signature: project.key_signature,
                    new_key_signature: Some(candidate),
                })
            };
            return ScalarAdoption {
                op,
                ops: Vec::new(),
                dropped: 0,
                reason: None,
            };
        }
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: read,
            reason: Some(format!(
                "key-signature-out-of-model-range: the file's single key signature is sharps_flats={sharps_flats}, outside -7..=7; project.key_signature untouched"
            )),
        };
    }
    let mut values = Vec::new();
    for (tick, &(sharps_flats, is_minor)) in &effective {
        values.push(format!("{sharps_flats} sf (minor={is_minor}) @{tick}"));
    }
    ScalarAdoption {
        op: None,
        ops: Vec::new(),
        dropped: read,
        reason: Some(format!(
            "key-signature-map-not-representable: {}; project.key_signature is a single scalar ⇒ a key-signature map has no home",
            values.join("; ")
        )),
    }
}

/// `FF 03` 轨名采纳计算（同 [`adopt_track_name`]）。
#[inline]
fn track_name_adoption_op(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    track_id: EntityId,
) -> ScalarAdoption {
    adopt_track_name(parsed, project, track_id)
}

/// `FF 03` 轨名 → `TrackV3::name`（导入的**唯一**落点音轨）。
///
/// **去重规则**：把所有 `MTrk` 的轨名当**一个集合**看；集合里**恰一个**非空、
/// 严格 UTF-8 名字时采用它（多个 `MTrk` 同名 = 同一个名字，不算冲突）。
/// 多个不同名字 / 空名 / 非 UTF-8 一律**不挑**（⛔ 拿第一条冒充整份文件）。
/// **不做截断**：模型的 `name` 是 `String`，没有长度上限 ⇒ 字节原样保留。
pub fn adopt_track_name(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    track_id: EntityId,
) -> ScalarAdoption {
    let events: &[ParsedTrackName] = &parsed.track_names;
    if events.is_empty() {
        return ScalarAdoption {
            op: None,
            ops: Vec::new(),
            dropped: 0,
            reason: None,
        };
    }
    let distinct: BTreeSet<&str> = events
        .iter()
        .filter_map(|event| event.text.as_deref())
        .filter(|text| !text.is_empty())
        .collect();
    let empty = events
        .iter()
        .filter(|event| event.text.as_deref() == Some(""))
        .count();
    let non_utf8 = events.iter().filter(|event| event.text.is_none()).count();
    if let (1, Some(name)) = (distinct.len(), distinct.iter().next()) {
        let name = *name;
        let landed = events
            .iter()
            .filter(|event| event.text.as_deref() == Some(name))
            .count();
        let dropped = events.len() - landed;
        let reason = if dropped == 0 {
            None
        } else {
            let mut parts = Vec::new();
            if empty > 0 {
                parts.push(format!("{empty} empty track-name event(s)"));
            }
            if non_utf8 > 0 {
                parts.push(format!("{non_utf8} non-UTF-8 track-name event(s)"));
            }
            Some(format!(
                "track-name-not-representable: {}; the single project track keeps the name \
                 `{name}` (empty / non-UTF-8 names are never adopted)",
                parts.join(" + ")
            ))
        };
        let current = project
            .tracks
            .get(&track_id)
            .map(|track| track.name.clone())
            .unwrap_or_default();
        let op = if current == name {
            None
        } else {
            Some(Op::SetTrackName {
                track_id,
                old_name: current,
                new_name: name.to_owned(),
            })
        };
        return ScalarAdoption {
            op,
            ops: Vec::new(),
            dropped,
            reason,
        };
    }
    let mut parts = Vec::new();
    if !distinct.is_empty() {
        parts.push(format!(
            "{} distinct names [{}]",
            distinct.len(),
            distinct
                .iter()
                .map(|name| format!("`{name}`"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if empty > 0 {
        parts.push(format!("{empty} empty name(s)"));
    }
    if non_utf8 > 0 {
        parts.push(format!("{non_utf8} non-UTF-8 name(s)"));
    }
    ScalarAdoption {
        op: None,
        ops: Vec::new(),
        dropped: events.len(),
        reason: Some(format!(
            "track-name-not-representable: {}; one TrackV3::name cannot carry them all",
            parts.join(" + ")
        )),
    }
}

/// `FF 02` 版权元数据采纳尝试。
///
/// 模型中的 `project.author` / `project.title` 并不是版权声明载体，因此不予采纳。
#[inline]
#[allow(dead_code)]
fn copyright_adoption_op(_parsed: &ParsedMidi, _project: &YebanProjectV1) -> Option<Op> {
    None
}

/// 由 marker 派生的一个段落（tick 已换算到模型的 960 PPQ）。
#[derive(Clone, Debug, PartialEq, Eq)]
struct PlannedSection {
    /// marker 文本（⛔ 不发明名字；空文本就是空名字，模型与 schema 都允许）。
    name: String,
    /// 起始 tick（模型 PPQ）。
    start_tick: u64,
    /// 结束 tick（模型 PPQ；恒 `> start_tick`）。
    end_tick: u64,
}

/// `FF 06` marker → `SectionV3` 区间（**纯函数**）。
///
/// ## 区间规则（⛔ 不重叠是**构造保证**，不是事后检查）
///
/// 1. marker 按 tick **稳定**排序（同 tick 保持文件顺序）；
/// 2. 同一 tick 的多条只留**最后一条**（MIDI 的生效语义），更早的逐条计入 dropped；
/// 3. 相邻两个边界之间是一个段落；最后一个边界到 [`ParsedMidi::end_tick`]；
/// 4. `end <= start`（零长度：末尾 marker 与文件结束重合，或低 PPQ 缩放后塌陷）
///    ⇒ 该段落**落不下去**，逐条计入 dropped 并写明 tick；
/// 5. 非 UTF-8 的 marker **仍是边界**（前一段在它的 tick 结束），只丢名字；
/// 6. 因此产出的区间**两两不重叠**、且按 tick 升序 —— `SectionV3::validate`
///    只拒绝 `end <= start`，**不查重叠**，重叠只能靠这里的构造保证。
///
/// ⚠ marker 的 tick **不可能 < 0**（SMF 的 delta 是无符号数），也**不可能 >
/// `end_tick`**（`end_tick` 是全部事件 tick 的最大值）⇒ 边界检查就是第 4 条。
fn plan_sections(
    markers: &[ParsedMarker],
    end_tick: u64,
    ppq: u16,
) -> (Vec<PlannedSection>, usize, Option<String>) {
    if markers.is_empty() {
        return (Vec::new(), 0, None);
    }
    let mut sorted: Vec<&ParsedMarker> = markers.iter().collect();
    sorted.sort_by_key(|marker| marker.tick);
    // (tick, 该 tick 的全部 marker（文件顺序）)；组内最后一条是生效的边界。
    let mut groups: Vec<(u64, Vec<&ParsedMarker>)> = Vec::new();
    for marker in &sorted {
        match groups.last_mut() {
            Some((tick, events)) if *tick == marker.tick => events.push(marker),
            _ => groups.push((marker.tick, vec![marker])),
        }
    }

    let mut sections = Vec::new();
    let mut dropped = 0usize;
    let mut details = Vec::new();
    for (index, (tick, events)) in groups.iter().enumerate() {
        let winner = events.last().expect("分组非空");
        let next_tick = groups.get(index + 1).map_or(end_tick, |(next, _)| *next);
        let start = scale_ticks(*tick, ppq);
        let end = scale_ticks(next_tick, ppq);
        if events.len() > 1 {
            dropped += events.len() - 1;
            let superseded = events[..events.len() - 1]
                .iter()
                .map(|event| match event.text.as_deref() {
                    Some(name) => format!("`{name}`"),
                    None => "<non-UTF-8>".to_owned(),
                })
                .collect::<Vec<_>>()
                .join(", ");
            details.push(format!(
                "{} marker(s) @{} superseded by a later marker at the same tick: {}",
                events.len() - 1,
                tick,
                superseded
            ));
        }
        match (&winner.text, end > start) {
            (Some(name), true) => sections.push(PlannedSection {
                name: name.clone(),
                start_tick: start,
                end_tick: end,
            }),
            (Some(name), false) => {
                dropped += 1;
                details.push(format!(
                    "zero-length section @{} (`{name}`): file ticks {}..{} collapse to model \
                     ticks {start}..{end}",
                    tick, tick, next_tick
                ));
            }
            (None, _) => {
                dropped += 1;
                details.push(format!(
                    "marker @{} is not valid UTF-8: its tick still ends the previous section, \
                     but no SectionV3::name can be written",
                    tick
                ));
            }
        }
    }
    debug_assert_eq!(
        dropped,
        markers.len() - sections.len(),
        "每个 marker 要么落成一个段落、要么恰计入一次 dropped"
    );
    let reason = if dropped == 0 {
        None
    } else {
        Some(format!("marker-not-representable: {}", details.join("; ")))
    };
    (sections, dropped, reason)
}

/// ⭐ ADR-0015 D2：CC 7 / CC 10 ⇒ 音轨自动化泳道（`Op::SetAutomationLane`）。
///
/// ## 归属与换算
///
/// - **音轨归属**与音符走**同一条** `track_destinations` 规则（拆分开启时按 `MTrk`，
///   否则统一进目标轨）⇒ CC 与它所在那条轨的音符落在同一个 `track_id` 上；
/// - **数值换算**见 [`cc7_to_decibels`] / [`cc10_to_pan`]；tick 走 [`scale_ticks`]
///   （与音符同一个纯比例换算，⛔ 不量化）；
/// - **同一 tick 后到者覆盖**：`BTreeMap<tick, value>` 天然给出"文件序最后一条生效"，
///   点位此之后按 tick 递增，⛔ 一条泳道里同一 tick 不留两个点。
///
/// ## 工程里已有同目标泳道时
///
/// 导入**不清空**作者已有的曲线：把新点插进原泳道，只在 tick **重合**处覆盖那一个点
/// （被覆盖的旧点随 `old_lane` 一起进操作，撤销可原样恢复）。
///
/// ## 为什么一条泳道一个 `SetAutomationLane`
///
/// `Op::SetAutomationPoint` 每次只落一个点，且它建出来的是**隐式形状**的泳道；一条
/// 映射出的曲线要的是"一次写入、一次可撤销"的整体 ⇒ 用 `SetAutomationLane`（ADR-0015 D2）。
///
/// 返回 `(ops, attached)`：`attached` = **被映射的事件条数**（CC 7 + CC 10，覆盖也算），
/// 供损失分桶用 `read - attached` 报未映射的控制器（⚠ 不是"点的条数"：同 tick 覆盖会
/// 让点数少于事件数，而守恒判据数的是**事件**）。
fn plan_cc_lanes(
    parsed: &ParsedMidi,
    project: &YebanProjectV1,
    track_destinations: &BTreeMap<usize, (EntityId, EntityId)>,
    dest_track_id: EntityId,
) -> (Vec<Op>, usize) {
    // ① 折成 (目标 → tick → 值)：`BTreeMap` 的键序给出确定顺序，后到者覆盖同 tick。
    let mut curves: BTreeMap<AutomationTarget, BTreeMap<u64, f32>> = BTreeMap::new();
    let mut attached = 0usize;
    for cc in &parsed.control_changes {
        let track_id = track_destinations
            .get(&cc.track)
            .map_or(dest_track_id, |(id, _)| *id);
        let (target, value) = match cc.controller {
            CC_CHANNEL_VOLUME => (
                AutomationTarget::TrackVolume { track_id },
                cc7_to_decibels(cc.value),
            ),
            CC_PAN => (
                AutomationTarget::TrackPan { track_id },
                cc10_to_pan(cc.value),
            ),
            // 其余控制器：模型里没有对应目标 ⇒ 不映射（逐条计入 `dropped`）。
            _ => continue,
        };
        curves
            .entry(target)
            .or_default()
            .insert(scale_ticks(cc.tick, parsed.ppq), value);
        attached += 1;
    }

    // ② 每条泳道一个 op。
    let mut ops = Vec::with_capacity(curves.len());
    for (target, points) in curves {
        let existing = project
            .tracks
            .get(&target.track_id())
            .and_then(|track| track.automation_lanes.get(&target));
        let mut lane = existing.cloned().unwrap_or_else(|| AutomationLane {
            target,
            points: BTreeMap::new(),
            read_enabled: true,
            write_mode: AutomationWriteMode::Off,
            domain: None,
        });
        // 同 tick 的旧点被本次导入的那一个顶掉（其余点一个字不动）。
        lane.points
            .retain(|_, point| !points.contains_key(&point.tick));
        for (tick, value) in points {
            let id = derive_note_id();
            lane.points.insert(
                id,
                AutomationPoint {
                    id,
                    tick,
                    value,
                    curve: CurveType::Linear,
                },
            );
        }
        ops.push(Op::SetAutomationLane {
            target,
            old_lane: existing.cloned(),
            new_lane: lane,
        });
    }
    (ops, attached)
}

/// 无落点的类别**为什么**落不下去（稳定的一句话；MCP 可逐字引用）。
///
/// ⚠ 这与 [`MidiAuxEventKind`] 的变体文档是**同一个事实的两处表达**：变体文档给
/// 人读，这里给报告/MCP 读。改一边必须改另一边（`aux_reasons` 的值来自这里）。
fn static_loss_reason(kind: MidiAuxEventKind) -> &'static str {
    match kind {
        // 七类有落点：它们的动态原因由 helper 给；这里的文本只是兜底。
        // （弯音的兜底即 D2 的真实原因：非唯一发声的条数落不下去。）
        MidiAuxEventKind::Tempo => "tempo not adopted into project",
        MidiAuxEventKind::TimeSignature => "time signature not adopted into project",
        MidiAuxEventKind::TrackName => "track names cannot all land in one TrackV3::name",
        MidiAuxEventKind::Marker => "marker cannot become a SectionV3 (zero-length / non-UTF-8)",
        MidiAuxEventKind::KeySignature => {
            "key-signature map has no home: project.key_signature is a single scalar"
        }
        // ⭐ ADR-0015 D2：CC 7 / CC 10 有落点（音量/声相泳道）；这里的原因是
        // **其余**控制器落不下去 —— 不是"CC 一律没有落点"（那句话自本 ADR 起就是错的）。
        MidiAuxEventKind::ControlChange => {
            "a control change other than CC 7 (channel volume) and CC 10 (pan) has no \
             automation target to map onto (ADR-0015 D2: only those two controllers land \
             in a lane)"
        }
        MidiAuxEventKind::ProgramChange => {
            "TrackV3::sampler is a sampler file path, not a MIDI program number"
        }
        MidiAuxEventKind::PitchBend => {
            "a pitch bend whose track and channel have zero or several sounding notes at its \
             tick cannot be honestly attributed to one note (ADR-0014 D2: only a unique \
             sounding note adopts it)"
        }
        MidiAuxEventKind::PolyAftertouch => "the model has no poly-aftertouch field",
        MidiAuxEventKind::ChannelAftertouch => "the model has no channel-aftertouch field",
        MidiAuxEventKind::SysEx => "the model has no SysEx payload carrier",
        MidiAuxEventKind::Escape => "the model has no escape payload carrier",
        MidiAuxEventKind::Text => "the model has no text-meta field",
        MidiAuxEventKind::Copyright => "project.author/title are not a copyright notice",
        MidiAuxEventKind::InstrumentName => {
            "TrackV3::sampler is a path; the model has no instrument-name field"
        }
        MidiAuxEventKind::TrackNumber => "the model has no track-number field",
        MidiAuxEventKind::CuePoint => {
            "the model has no cue-point entity (cue points are not SectionV3 ranges)"
        }
        MidiAuxEventKind::ProgramName => "the model has no program-name field",
        MidiAuxEventKind::DeviceName => "the model has no device-name field",
        MidiAuxEventKind::MidiChannelPrefix => "MidiNote has no channel field",
        MidiAuxEventKind::MidiPort => "the model has no MIDI-port field",
        MidiAuxEventKind::SmpteOffset => "the model has no SMPTE-offset field",
        MidiAuxEventKind::SequencerSpecific => "the model has no sequencer-specific carrier",
        MidiAuxEventKind::UnknownMeta => {
            "unrecognised meta id: the model has no carrier and the parser does not guess \
             its meaning"
        }
    }
}

/// ⭐ `#317`：歌词归属计算 —— **唯一才归属**。
fn attach_lyrics(parsed: &ParsedMidi) -> (BTreeMap<u64, &str>, MidiLyricCounts) {
    let mut note_count_at: BTreeMap<u64, usize> = BTreeMap::new();
    for note in &parsed.notes {
        *note_count_at.entry(note.start_tick).or_insert(0) += 1;
    }
    let mut lyric_count_at: BTreeMap<u64, usize> = BTreeMap::new();
    for lyric in &parsed.lyrics {
        *lyric_count_at.entry(lyric.tick).or_insert(0) += 1;
    }
    let mut attachable: BTreeMap<u64, &str> = BTreeMap::new();
    for lyric in &parsed.lyrics {
        if note_count_at.get(&lyric.tick) == Some(&1) && lyric_count_at.get(&lyric.tick) == Some(&1)
        {
            attachable.insert(lyric.tick, lyric.text.as_str());
        }
    }
    let lyrics = MidiLyricCounts {
        read: parsed.lyrics.len(),
        attached: attachable.len(),
        dropped: parsed.lyrics.len() - attachable.len(),
        non_utf8: parsed.non_utf8_lyrics,
        empty: parsed.empty_lyrics,
    };
    (attachable, lyrics)
}

/// ⭐ ADR-0014 D2：弯音归属计算 —— 与歌词（`#317`）同一条**唯一才归属**原则。
///
/// 对每条 `ParsedPitchBend`，在**同轨同通道**上找发声区间
/// （`start_tick <= tick < end_tick`）覆盖它的音符：
///
/// - **恰一颗**（单音旋律线）⇒ 归属该音符，局部坐标
///   `rel_tick = scale_ticks(bend.tick) - scale_ticks(note.start_tick)`（ADR-0012
///   的音符局部 tick）；同 tick 后来者覆盖，序列保持 tick 严格升序；
/// - 0 颗（空档）或 >1 颗（复音共用通道）⇒ 不归属（记入丢失，⛔ 不猜）。
///
/// 返回与 `parsed.notes` **下标对齐**的逐音符弯音序列，以及归属成功的事件条数。
fn attach_pitch_bends(parsed: &ParsedMidi) -> (Vec<Vec<(u64, i16)>>, usize) {
    let mut curves: Vec<Vec<(u64, i16)>> = vec![Vec::new(); parsed.notes.len()];
    let mut attached = 0usize;
    for bend in &parsed.pitch_bends {
        let sounding: Vec<usize> = parsed
            .notes
            .iter()
            .enumerate()
            .filter(|(_, note)| {
                note.track == bend.track
                    && note.channel == bend.channel
                    && note.start_tick <= bend.tick
                    && bend.tick < note.end_tick
            })
            .map(|(index, _)| index)
            .collect();
        let [index] = sounding.as_slice() else {
            continue;
        };
        let rel_tick = scale_ticks(bend.tick, parsed.ppq)
            .saturating_sub(scale_ticks(parsed.notes[*index].start_tick, parsed.ppq));
        let curve = &mut curves[*index];
        match curve.binary_search_by_key(&rel_tick, |&(tick, _)| tick) {
            Ok(position) => curve[position].1 = bend.cents,
            Err(position) => curve.insert(position, (rel_tick, bend.cents)),
        }
        attached += 1;
    }
    (curves, attached)
}

/// 丢失统计计算（按类汇总 `aux_events` 及通道/力度损失）。
#[allow(clippy::too_many_arguments)]
fn count_losses(
    parsed: &ParsedMidi,
    notes: usize,
    tempo: &ScalarAdoption,
    time_signature: &ScalarAdoption,
    track_name: &ScalarAdoption,
    key_signature: &ScalarAdoption,
    marker: (usize, Option<String>),
    track_partitions_dropped: usize,
    pitch_bends_attached: usize,
    cc_attached: usize,
) -> MidiImportLosses {
    let (marker_dropped, marker_reason) = marker;
    let mut aux_events: BTreeMap<MidiAuxEventKind, MidiLossCount> = BTreeMap::new();
    let mut aux_reasons: BTreeMap<MidiAuxEventKind, String> = BTreeMap::new();
    let stored = [
        (MidiAuxEventKind::Tempo, tempo),
        (MidiAuxEventKind::TimeSignature, time_signature),
        (MidiAuxEventKind::TrackName, track_name),
        (MidiAuxEventKind::KeySignature, key_signature),
    ];
    for kind in MidiAuxEventKind::ALL {
        let read = parsed.aux_events.count(kind);
        let (dropped, reason) = if kind == MidiAuxEventKind::Marker {
            (marker_dropped, marker_reason.clone())
        } else if kind == MidiAuxEventKind::PitchBend {
            // ⭐ ADR-0014 D2：唯一发声归属的弯音进了音符，其余报丢（read == attached + dropped）。
            (read.saturating_sub(pitch_bends_attached), None)
        } else if kind == MidiAuxEventKind::ControlChange {
            // ⭐ ADR-0015 D2：CC 7 / CC 10 进了自动化泳道，其余控制器报丢
            // （read == attached + dropped）。
            (read.saturating_sub(cc_attached), None)
        } else if let Some((_, adoption)) = stored.iter().find(|(stored, _)| *stored == kind) {
            (adoption.dropped, adoption.reason.clone())
        } else {
            (read, None)
        };
        debug_assert!(
            dropped <= read,
            "{}: dropped={dropped} > read={read}（⛔ 丢掉的事件数不可能超过读到的）",
            kind.key()
        );
        aux_events.insert(kind, MidiLossCount { read, dropped });
        if dropped > 0 {
            aux_reasons.insert(
                kind,
                reason.unwrap_or_else(|| static_loss_reason(kind).to_owned()),
            );
        }
    }
    let distinct_channels = parsed
        .notes
        .iter()
        .map(|note| note.channel)
        .collect::<BTreeSet<u8>>()
        .len();
    MidiImportLosses {
        format: parsed.format,
        file_tracks: parsed.tracks,
        track_partitions_dropped,
        distinct_channels,
        // 每颗音符都带一个通道值，而 `MidiNote` 没有 channel 字段 ⇒ 全丢。
        note_channel_values: MidiLossCount {
            read: notes,
            dropped: if channel_is_stored_on_note() {
                0
            } else {
                notes
            },
        },
        // 每颗音符都有一条关闭事件，它的释放力度被解析器忽略 ⇒ 全丢。
        note_off_velocity_values: MidiLossCount {
            read: notes,
            dropped: notes,
        },
        aux_events,
        aux_reasons,
    }
}

/// ADR-0013 D1/D2：多轨拆分新建音轨的命名回退链。
///
/// 优先级：`FF 03` 轨名 → `FF 04` 乐器名 → `FF 08` 程序名 → 机器名
/// `Track {n}`。每一类内：该 `MTrk` 上**去重后恰一个**非空、严格 UTF-8 的名字
/// 才算有效；出现多个互不相同的同类名字 ⇒ 判为歧义，⛔ 不随意挑选，
/// 回退到下一个优先级（D2）。
fn split_track_name(parsed: &ParsedMidi, track_index: usize) -> String {
    for events in [
        &parsed.track_names,
        &parsed.instrument_names,
        &parsed.program_names,
    ] {
        let mut candidates: BTreeSet<&str> = BTreeSet::new();
        for event in events {
            if event.track != track_index {
                continue;
            }
            if let Some(text) = event.text.as_deref().filter(|s| !s.is_empty()) {
                candidates.insert(text);
            }
        }
        if candidates.len() == 1 {
            return candidates
                .into_iter()
                .next()
                .expect("非空集合必有唯一元素")
                .to_owned();
        }
    }
    format!("Track {}", track_index + 1)
}

/// **纯映射**：`ParsedMidi` ⇒ 一批 `Op`（⛔ 不改工程、⛔ 不提交、⛔ 不碰文件系统）。
///
/// 产物是**结构化**的：`ops`（音符 + `SetTempo` / `SetTimeSignature` /
/// `SetTrackName` / `SetSection`）与 [`MidiImportLosses`]（含逐类
/// [`MidiImportLosses::aux_reasons`]）。⛔ 不打印、⛔ 不读路径 ——
/// `yeban_import_midi` 工具可以直接序列化它。
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
    let (dest_track_id, dest_clip_id) = destination(project, options)?;

    let (attachable, lyrics) = attach_lyrics(parsed);
    // ⭐ ADR-0014 D2：弯音的逐音符归属（唯一发声才归属）。
    let (bend_curves, bends_attached) = attach_pitch_bends(parsed);

    let note_tracks: Vec<usize> = {
        let mut set = BTreeSet::new();
        for note in &parsed.notes {
            set.insert(note.track);
        }
        set.into_iter().collect()
    };

    let mut ops: Vec<Op> = Vec::with_capacity(parsed.notes.len() + note_tracks.len() * 5);

    let mut track_destinations: BTreeMap<usize, (EntityId, EntityId)> = BTreeMap::new();
    let first_track = note_tracks[0];
    track_destinations.insert(first_track, (dest_track_id, dest_clip_id));

    if options.split_tracks {
        for &track_index in &note_tracks[1..] {
            let new_track_id = derive_note_id();
            let new_clip_id = derive_note_id();
            let placement_id = derive_note_id();
            let edge_id = derive_note_id();

            // ⭐ ADR-0013：D1/D2 回退链（FF 03 → FF 04 → FF 08 → 机器名）。
            let track_name = split_track_name(parsed, track_index);

            let track_duration = parsed
                .notes
                .iter()
                .filter(|n| n.track == track_index)
                .map(|n| scale_ticks(n.end_tick.max(n.start_tick + 1), parsed.ppq))
                .max()
                .unwrap_or(MODEL_PPQ)
                .max(1);

            let clip_name = track_name.clone();

            ops.push(Op::AddTrack {
                track: TrackV3 {
                    id: new_track_id,
                    name: track_name,
                    kind: TrackKind::Midi,
                    ..TrackV3::default()
                },
            });
            ops.push(Op::AddRoutingNode { node: new_track_id });
            if !project
                .routing_graph
                .nodes
                .contains(&project.master_bus_track_id)
            {
                ops.push(Op::AddRoutingNode {
                    node: project.master_bus_track_id,
                });
            }
            ops.push(Op::AddClip {
                clip: ClipPoolEntry {
                    id: new_clip_id,
                    name: clip_name,
                    content: ClipContent::Midi {
                        notes: BTreeMap::new(),
                    },
                },
            });
            ops.push(Op::AddClipPlacement {
                track_id: new_track_id,
                placement: ClipPlacement {
                    id: placement_id,
                    clip_id: new_clip_id,
                    start_tick: 0,
                    duration_ticks: track_duration,
                    ..ClipPlacement::default()
                },
            });
            ops.push(Op::ConnectRouting {
                edge: RoutingEdge {
                    id: edge_id,
                    source_node: new_track_id,
                    destination_node: project.master_bus_track_id,
                    kind: RoutingKind::TrackToBus,
                    gain_db: None,
                },
            });

            track_destinations.insert(track_index, (new_track_id, new_clip_id));
        }
    }

    ops.extend(parsed.notes.iter().enumerate().map(|(note_index, note)| {
        let (target_track_id, target_clip_id) = track_destinations
            .get(&note.track)
            .copied()
            .unwrap_or((dest_track_id, dest_clip_id));

        let start = scale_ticks(note.start_tick, parsed.ppq);
        let end = scale_ticks(note.end_tick.max(note.start_tick + 1), parsed.ppq);
        let mut model_note = MidiNote::new(
            derive_note_id(),
            start,
            note.key,
            end.saturating_sub(start).max(1),
        );
        model_note.velocity = note.velocity;
        if channel_is_stored_on_note() {
            model_note.channel = Some(note.channel);
        }
        if let Some(text) = attachable.get(&note.start_tick) {
            model_note.syllable = Some((*text).to_owned());
        }
        model_note.pitch_bend_curve = bend_curves[note_index].clone();
        Op::AddNote {
            track_id: target_track_id,
            clip_id: target_clip_id,
            note: model_note,
        }
    }));
    let notes = parsed.notes.len();

    // ⭐ ADR-0015 D2：CC 7 / CC 10 ⇒ 自动化泳道。排在**新建轨 ops 之后**（拆分开启时
    // `SetAutomationLane` 的 `old_lane` 前置检查要求目标音轨已存在），⛔ 不新建音轨。
    let (lane_ops, cc_attached) =
        plan_cc_lanes(parsed, project, &track_destinations, dest_track_id);
    ops.extend(lane_ops);

    // 分桶自检：既不重叠也不漏（`read == attached + dropped`）。
    debug_assert!(
        lyrics.is_consistent(),
        "歌词分桶必须满足 read == attached + dropped: {lyrics:?}"
    );

    // ⭐ `#342`：四类有落点的元数据各走一个**纯** helper（只读 `parsed` + `project`）；
    // 音符 ops 先入队，元数据 ops 后入队。
    let target_empty = import_target_is_empty(project);
    let mut tempo = tempo_adoption_op(parsed, project, target_empty);
    let mut time_signature = time_signature_adoption_op(parsed, project, target_empty);
    let mut track_name = track_name_adoption_op(parsed, project, dest_track_id);
    let mut key_signature = key_signature_adoption_op(parsed, project, target_empty);
    let (sections, marker_dropped, marker_reason) =
        plan_sections(&parsed.markers, parsed.end_tick, parsed.ppq);
    for adoption in [
        &mut track_name,
        &mut tempo,
        &mut time_signature,
        &mut key_signature,
    ] {
        ops.extend(adoption.all_ops());
    }
    for section in &sections {
        let section_id = derive_note_id();
        ops.push(Op::SetSection {
            section_id,
            old_section: None,
            new_section: SectionV3 {
                id: section_id,
                name: section.name.clone(),
                start_tick: section.start_tick,
                end_tick: section.end_tick,
                color: None,
            },
        });
    }

    let track_partitions_dropped = if options.split_tracks {
        0
    } else {
        parsed.tracks.saturating_sub(1)
    };

    let losses = count_losses(
        parsed,
        notes,
        &tempo,
        &time_signature,
        &track_name,
        &key_signature,
        (marker_dropped, marker_reason),
        track_partitions_dropped,
        bends_attached,
        cc_attached,
    );
    debug_assert!(
        losses.is_consistent(),
        "丢失读数必须满足 dropped <= read 且轨分区 <= file_tracks - 1: {losses:?}"
    );
    debug_assert!(
        losses.is_total_loss(),
        "无落点的类别必须仍全损（有落点的四类不受这条约束；⛔ 别删判据）: {losses:?}"
    );
    Ok(MidiImportPlan {
        ops,
        notes,
        track_id: dest_track_id,
        clip_id: dest_clip_id,
        ppq: parsed.ppq,
        lyrics,
        losses,
    })
}

/// 选目标：第一条非 master 的音轨 ＋ 它的第一个摆放。
///
/// ⚠ `pub`：`yeban-app::midi_notes` 的 `--midi-in` 写音符与 MCP 导入音轨走**同一条**规则
/// （⛔ 不许两处各写一份"第一条非 master 音轨"）。
pub fn destination(
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

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::midi::{
        MidiAuxEventCounts, MidiAuxEventKind, MidiFormat, MidiTempo, ParsedLyric, ParsedNote,
    };

    /// 一段**最小合法 SMF**（格式 0、960 PPQ、一个 C4 音符）。
    pub fn minimal_smf() -> Vec<u8> {
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

    /// 造一份**只差 tempo** 的最小 SMF：1 条轨、960 PPQ、一个 C4 音符、⭐ 指定的
    /// `(tick, µs/四分音符)` 序列（tick 是全轨绝对 tick）。
    pub fn smf_with_tempos(tempos: &[(u32, u32)]) -> Vec<u8> {
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
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: Vec::new(),
            instrument_names: Vec::new(),
            program_names: Vec::new(),
            markers: Vec::new(),
            end_tick: 0,
            notes,
            lyrics,
            non_utf8_lyrics,
            empty_lyrics,
        }
    }

    /// 往夹具的 aux 计数里记 `count` 条 `kind`（测试只关心"读到了几条"）。
    fn recording(mut parsed: ParsedMidi, kind: MidiAuxEventKind, count: usize) -> ParsedMidi {
        for _ in 0..count {
            parsed.aux_events.record(kind);
        }
        parsed
    }

    fn marker(tick: u64, text: Option<&str>) -> ParsedMarker {
        ParsedMarker {
            tick,
            text: text.map(str::to_owned),
        }
    }

    fn track_name(track: usize, tick: u64, text: Option<&str>) -> ParsedTrackName {
        ParsedTrackName {
            track,
            tick,
            text: text.map(str::to_owned),
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
            track: 0,
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

    /// `scale_ticks` 的整数比例换算与截断特性。
    #[test]
    fn scale_ticks_proportional_and_truncates() {
        assert_eq!(scale_ticks(0, 480), 0);
        assert_eq!(scale_ticks(480, 480), 960);
        assert_eq!(scale_ticks(240, 480), 480);
        assert_eq!(scale_ticks(960, 960), 960);
        assert_eq!(scale_ticks(1920, 1920), 960);
        assert_eq!(scale_ticks(100, 300), 320); // 100 * 960 / 300 = 320
        assert_eq!(scale_ticks(100, 0), 96_000); // ppq.max(1)
    }

    /// `STORABLE_KINDS` 与 `STORABLE_KIND_KEYS` 的一致性契约。
    #[test]
    fn storable_kinds_consistency() {
        // ⭐ ADR-0015 D2：`ControlChange` 自 6 项起加入可落点集合（7 项）。
        assert_eq!(STORABLE_KINDS.len(), 7);
        assert_eq!(STORABLE_KIND_KEYS.len(), 7);
        for (kind, &key) in STORABLE_KINDS.iter().zip(STORABLE_KIND_KEYS) {
            assert_eq!(kind.key(), key);
        }
    }

    /// `MidiImportLosses` 的 `is_empty`、`loss_total` 与 `to_json`。
    #[test]
    fn losses_methods_is_empty_loss_total_and_to_json() {
        let empty_losses = MidiImportLosses {
            format: MidiFormat::SingleTrack,
            file_tracks: 1,
            track_partitions_dropped: 0,
            distinct_channels: 1,
            note_channel_values: MidiLossCount {
                read: 0,
                dropped: 0,
            },
            note_off_velocity_values: MidiLossCount {
                read: 0,
                dropped: 0,
            },
            aux_events: BTreeMap::new(),
            aux_reasons: BTreeMap::new(),
        };
        assert!(empty_losses.is_empty());
        assert_eq!(empty_losses.loss_total(), 0);

        let json = empty_losses.to_json();
        assert!(json.contains(r#""loss_total":0"#));
        assert!(json.contains(r#""format":0"#));
        assert!(json.contains(r#""file_tracks":1"#));

        let non_empty = all_kinds_losses();
        assert!(!non_empty.is_empty());
        assert!(non_empty.loss_total() > 0);
        let json2 = non_empty.to_json();
        assert!(json2.contains(r#""loss_total":"#));
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

    /// CC 事件夹具（`track` / `channel` 恒 0：ADR-0015 D2 的归属只看**轨道**）。
    fn cc(tick: u64, controller: u8, value: u8) -> crate::midi::ParsedControlChange {
        cc_on(0, tick, controller, value)
    }

    /// 同上，另指定 `MTrk` 序号。
    fn cc_on(
        track: usize,
        tick: u64,
        controller: u8,
        value: u8,
    ) -> crate::midi::ParsedControlChange {
        crate::midi::ParsedControlChange {
            track,
            channel: 0,
            tick,
            controller,
            value,
        }
    }

    /// plan 里的泳道 op 摘成 `(目标, 新泳道, 改动前的泳道)`，保持发出顺序。
    fn lane_ops(
        plan: &MidiImportPlan,
    ) -> Vec<(AutomationTarget, AutomationLane, Option<AutomationLane>)> {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                Op::SetAutomationLane {
                    target,
                    old_lane,
                    new_lane,
                } => Some((*target, new_lane.clone(), old_lane.clone())),
                _ => None,
            })
            .collect()
    }

    /// 泳道点位折成按 tick 升序的 `(tick, value)`（`points` 以身份为键 ⇒ 必须自己排序）。
    fn lane_points(lane: &AutomationLane) -> Vec<(u64, f32)> {
        let mut rows: Vec<(u64, f32)> = lane
            .points
            .values()
            .map(|point| (point.tick, point.value))
            .collect();
        rows.sort_by_key(|row| row.0);
        rows
    }

    /// ⭐ ADR-0015 D2 换算表：期望值是**字面读数**（⛔ 不与公式自比）。
    #[test]
    fn cc7_and_cc10_conversions_match_the_adr_literals() {
        assert_eq!(
            cc7_to_decibels(0),
            -60.0,
            "v = 0 走专用下限（log10(0) 是 -inf）"
        );
        assert_eq!(cc7_to_decibels(127), 0.0, "满格 ⇒ 0 dB");
        assert!(
            (cc7_to_decibels(100) - (-4.152_148_7)).abs() < 1e-3,
            "GM 默认音量 CC 7 = 100 ⇒ 约 -4.152 dB，实得 {}",
            cc7_to_decibels(100)
        );
        assert_eq!(
            cc7_to_decibels(1),
            -60.0,
            "40·log10(1/127) = -84.15 ⇒ 必须被钳到值域下限 -60"
        );
        assert_eq!(cc10_to_pan(0), -1.0, "极左");
        assert_eq!(cc10_to_pan(64), 0.0, "居中不偏");
        assert_eq!(cc10_to_pan(127), 1.0, "极右（右半段宽 63 ⇒ 恰好 1.0）");
        assert_eq!(CC_CHANNEL_VOLUME, 7);
        assert_eq!(CC_PAN, 10);
    }

    /// ⭐ ADR-0015 D2 判据 a（CC 10 ⇒ **新建**声相泳道）：目标轨原本没有 `TrackPan`
    /// 泳道 ⇒ `old_lane = None`、三点落进同一条泳道、`read == attached` 一条不丢。
    #[test]
    fn a_cc10_event_becomes_a_new_track_pan_lane() {
        let project = yeban_model::samples::filled_project();
        let (dest_track_id, _) =
            destination(&project, &MidiImportOptions::default()).expect("目标轨");
        let mut parsed = parsed(960, vec![note(60, 100, 0, 960)]);
        parsed.control_changes = vec![cc(0, CC_PAN, 0), cc(480, CC_PAN, 64), cc(960, CC_PAN, 127)];
        let read = parsed.control_changes.len();
        let parsed = recording(parsed, MidiAuxEventKind::ControlChange, read);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");

        let rows = lane_ops(&plan);
        assert_eq!(rows.len(), 1, "三个 CC 10 同目标 ⇒ 一条泳道 op");
        let (target, lane, old) = &rows[0];
        assert_eq!(
            *target,
            AutomationTarget::TrackPan {
                track_id: dest_track_id
            },
            "声相必须挂在**导入目标轨**上"
        );
        assert_eq!(*old, None, "目标轨原本没有声相泳道");
        assert_eq!(
            lane_points(lane),
            vec![(0, -1.0), (480, 0.0), (960, 1.0)],
            "960 PPQ ⇒ tick 一比一"
        );
        assert!(
            !lane.is_implicit(),
            "带点的泳道必须不是隐式形状 ⇒ `SetAutomationLane` 的前置条件"
        );
        assert!(lane.read_enabled, "新建泳道默认开启读通");
        assert_eq!(lane.write_mode, AutomationWriteMode::Off);
        assert_eq!(lane.domain, None);
        assert_eq!(
            lane.points
                .values()
                .map(|point| point.curve)
                .collect::<Vec<CurveType>>(),
            vec![CurveType::Linear; 3],
            "导入点一律线性（ADR-0015 D2 不猜曲线形状）"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::ControlChange],
            MidiLossCount {
                read: 3,
                dropped: 0
            },
            "CC 10 有落点 ⇒ 一条不丢"
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::ControlChange),
            "没丢的类别不许编原因"
        );
    }

    /// ⭐ ADR-0015 D2 判据 b（CC 7 ⇒ **并入**作者已有的音量泳道）：`filled_project` 的
    /// 首轨本来就带一条 `TrackVolume` 泳道（tick 0 = `-6.0`、tick 3840 = `0.0` / SCurve）。
    /// 导入只顶掉 tick **重合**的那一个点，其余连曲线形状一起保留，且 `old_lane` 必须
    /// 带出改动前的整条泳道（撤销要能原样恢复）。
    #[test]
    fn a_cc7_event_merges_into_the_authored_volume_lane() {
        let project = yeban_model::samples::filled_project();
        let (dest_track_id, _) =
            destination(&project, &MidiImportOptions::default()).expect("目标轨");
        let target = AutomationTarget::TrackVolume {
            track_id: dest_track_id,
        };
        let authored = project.tracks[&dest_track_id]
            .automation_lanes
            .get(&target)
            .expect("夹具的音量泳道")
            .clone();
        assert_eq!(lane_points(&authored), vec![(0, -6.0), (3840, 0.0)]);

        let mut parsed = parsed(960, vec![note(60, 100, 0, 960)]);
        parsed.control_changes = vec![
            cc(0, CC_CHANNEL_VOLUME, 127),
            cc(960, CC_CHANNEL_VOLUME, 100),
        ];
        let read = parsed.control_changes.len();
        let parsed = recording(parsed, MidiAuxEventKind::ControlChange, read);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");

        let rows = lane_ops(&plan);
        assert_eq!(rows.len(), 1);
        let (got_target, lane, old) = &rows[0];
        assert_eq!(*got_target, target);
        assert_eq!(
            old.as_ref(),
            Some(&authored),
            "撤销信息必须是改动前的整条泳道"
        );
        let points = lane_points(lane);
        assert_eq!(
            points.iter().map(|row| row.0).collect::<Vec<u64>>(),
            vec![0, 960, 3840],
            "tick 0 被覆盖、tick 960 新增、作者的 tick 3840 保留"
        );
        assert_eq!(points[0].1, 0.0, "CC 7 = 127 ⇒ 0 dB（顶掉作者的 -6.0）");
        assert!(
            (points[1].1 - (-4.152_148_7)).abs() < 1e-3,
            "CC 7 = 100 ⇒ 约 -4.152 dB，实得 {}",
            points[1].1
        );
        assert_eq!(points[2].1, 0.0, "作者原点一个字没动");
        assert_eq!(
            lane.points
                .values()
                .filter(|point| point.tick == 3840)
                .map(|point| point.curve)
                .collect::<Vec<CurveType>>(),
            vec![CurveType::SCurve],
            "未被覆盖的作者点连曲线形状一起留下"
        );
        assert_eq!(lane.write_mode, authored.write_mode, "泳道级字段沿用作者的");
        assert_eq!(lane.domain, authored.domain);
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::ControlChange],
            MidiLossCount {
                read: 2,
                dropped: 0
            }
        );

        // 模型侧闭环：`SetAutomationLane` 的三条前置条件（目标一致 / 非隐式 /
        // `old_lane` 与文档现状相等）必须由 `apply` 真的接受。
        let mut doc = project.clone();
        for op in plan
            .ops
            .iter()
            .filter(|op| matches!(op, Op::SetAutomationLane { .. }))
        {
            op.apply(&mut doc).expect("模型必须接受本次导入的泳道 op");
        }
        assert_eq!(
            lane_points(
                doc.tracks[&dest_track_id]
                    .automation_lanes
                    .get(&target)
                    .expect("落地后的泳道")
            )
            .iter()
            .map(|row| row.0)
            .collect::<Vec<u64>>(),
            vec![0, 960, 3840],
            "落进文档的点位与规划一致"
        );
    }

    /// ⭐ ADR-0015 D2 判据 c（同 tick 后到者覆盖）：同一 tick 两条 CC 10 ⇒ 只留**文件序**
    /// 最后那条的值。⚠ `attached` 数的是**事件**（2），点数是 1 ⇒ 守恒式仍成立。
    #[test]
    fn a_later_control_change_on_the_same_tick_overrides_the_earlier_one() {
        let project = yeban_model::samples::filled_project();
        let (dest_track_id, _) =
            destination(&project, &MidiImportOptions::default()).expect("目标轨");
        let mut parsed = parsed(960, vec![note(60, 100, 0, 960)]);
        parsed.control_changes = vec![cc(0, CC_PAN, 0), cc(0, CC_PAN, 127)];
        let read = parsed.control_changes.len();
        let parsed = recording(parsed, MidiAuxEventKind::ControlChange, read);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");

        let rows = lane_ops(&plan);
        assert_eq!(rows.len(), 1, "同目标同 tick ⇒ 一条泳道里的一个点");
        assert_eq!(
            rows[0].0,
            AutomationTarget::TrackPan {
                track_id: dest_track_id
            },
            "覆盖发生在同一条目标轨声相泳道上"
        );
        assert_eq!(lane_points(&rows[0].1), vec![(0, 1.0)], "后到的 127 生效");
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::ControlChange],
            MidiLossCount {
                read: 2,
                dropped: 0
            },
            "被覆盖的那条也算**落点了**（守恒式数的是事件，不是点）"
        );
    }

    /// ⭐ ADR-0015 D2 判据 d（未映射的控制器报丢）：CC 1（调制）与 CC 64（延音踏板）
    /// 在模型里没有对应目标 ⇒ ⛔ 不新建泳道、逐条计入 `dropped`、原因走静态文案。
    #[test]
    fn an_unsupported_control_change_is_reported_as_dropped() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = parsed(960, vec![note(60, 100, 0, 960)]);
        parsed.control_changes = vec![cc(0, 1, 50), cc(480, 64, 127)];
        let read = parsed.control_changes.len();
        let parsed = recording(parsed, MidiAuxEventKind::ControlChange, read);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");

        assert!(lane_ops(&plan).is_empty(), "未映射的控制器不许造出泳道 op");
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::ControlChange],
            MidiLossCount {
                read: 2,
                dropped: 2
            },
            "read == attached + dropped（attached = 0）"
        );
        assert_eq!(
            plan.losses.aux_reasons[&MidiAuxEventKind::ControlChange],
            static_loss_reason(MidiAuxEventKind::ControlChange),
            "只有 CC 7 / CC 10 有落点 ⇒ 其余走静态兜底文案"
        );
    }

    /// ⭐ ADR-0015 D2 判据 e（拆分归属）：`split_tracks = true` 时第二条 `MTrk` 的 CC
    /// 必须落进**那条轨新建出来的**音轨，且泳道 op 排在它的 `AddTrack` **之后**
    /// （前置条件要求目标音轨此刻存在）。
    #[test]
    fn a_cc_on_a_split_track_lands_on_the_new_tracks_lane() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = parsed(
            960,
            vec![
                note(60, 100, 0, 960),
                ParsedNote {
                    track: 1,
                    ..note(64, 100, 0, 960)
                },
            ],
        );
        parsed.control_changes = vec![cc_on(1, 0, CC_PAN, 0)];
        let read = parsed.control_changes.len();
        let parsed = recording(parsed, MidiAuxEventKind::ControlChange, read);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");

        let rows = lane_ops(&plan);
        assert_eq!(rows.len(), 1);
        let (target, _, old) = &rows[0];
        assert_eq!(*old, None, "新建出来的音轨不可能带泳道");
        let AutomationTarget::TrackPan { track_id } = *target else {
            panic!("CC 10 必须落在声相目标上，实得 {target:?}");
        };
        assert_ne!(
            track_id,
            destination(&project, &MidiImportOptions::default())
                .expect("目标轨")
                .0,
            "第二条 `MTrk` 的 CC 不许并进首轨的泳道"
        );

        let add_index = plan
            .ops
            .iter()
            .rposition(|op| matches!(op, Op::AddTrack { track } if track.id == track_id))
            .expect("拆分必须先建轨");
        let lane_index = plan
            .ops
            .iter()
            .rposition(|op| matches!(op, Op::SetAutomationLane { .. }))
            .expect("必须有泳道 op");
        assert!(lane_index > add_index, "泳道 op 必须排在 `AddTrack` 之后");

        let mut doc = project.clone();
        for op in plan.ops.iter().take(lane_index + 1) {
            op.apply(&mut doc)
                .expect("建轨 → 落泳道 的整条前缀必须可应用");
        }
        assert!(
            doc.tracks[&track_id].automation_lanes.contains_key(target),
            "新建音轨在文档里确实拿到了这条泳道"
        );
    }

    /// 一条首轨首通道的弯音事件夹具（ADR-0014 D2 判据专用）。
    fn bend(tick: u64, cents: i16) -> crate::midi::ParsedPitchBend {
        crate::midi::ParsedPitchBend {
            track: 0,
            channel: 0,
            tick,
            cents,
        }
    }

    /// 把 `AddNote` 折成 `(起始 tick, 弯音曲线)` 并按起点排序。
    fn bend_curves(plan: &MidiImportPlan) -> Vec<(u64, Vec<(u64, i16)>)> {
        let mut rows: Vec<(u64, Vec<(u64, i16)>)> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::AddNote { note, .. } => Some((note.start_tick, note.pitch_bend_curve.clone())),
                _ => None,
            })
            .collect();
        rows.sort_by_key(|row| row.0);
        rows
    }

    /// ⭐ ADR-0014 D2 判据 a（单音归属）：同轨同通道**不重叠**的音符序列，每条弯音
    /// 恰好落进一颗音符 ⇒ `rel_tick = scale_ticks(bend) - scale_ticks(note.start)`
    /// 逐点挂进 `pitch_bend_curve`（480 PPQ ⇒ tick 全部乘 2），`dropped == 0`。
    #[test]
    fn a_pitch_bend_over_a_unique_sounding_note_attaches_with_note_local_ticks() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = parsed(480, vec![note(60, 100, 0, 480), note(64, 100, 480, 960)]);
        parsed.pitch_bends = vec![
            bend(0, 120),    // 第一颗：rel 0*2 = 0
            bend(120, 200),  // 第一颗：rel (240-0) = 240
            bend(480, 800),  // 第二颗起点（第一颗 end=480 **不含**）：rel 0
            bend(720, -400), // 第二颗：rel (1440-960) = 480
        ];
        let bends = parsed.pitch_bends.len();
        let parsed = recording(parsed, MidiAuxEventKind::PitchBend, bends);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("必须有目标");
        assert_eq!(
            bend_curves(&plan),
            vec![
                (0, vec![(0, 120), (240, 200)]),
                (960, vec![(0, 800), (480, -400)]),
            ],
            "弯音必须挂到唯一发声的那颗音符上，坐标是音符局部 tick"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::PitchBend],
            MidiLossCount {
                read: 4,
                dropped: 0,
            },
            "全部归属 ⇒ 一条不丢"
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::PitchBend),
            "没丢的类别不许编原因"
        );
    }

    /// ⭐ ADR-0014 D2 判据 b（空档不归属）：弯音 tick 不在任何发声区间 ⇒ 不归属，
    /// 逐条计入 `dropped`，原因用静态兜底文案。
    #[test]
    fn a_pitch_bend_in_a_gap_between_notes_is_dropped() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = parsed(960, vec![note(60, 100, 0, 480)]);
        parsed.pitch_bends = vec![bend(600, 50)]; // 600 在第一颗 end=480 之后
        let bends = parsed.pitch_bends.len();
        let parsed = recording(parsed, MidiAuxEventKind::PitchBend, bends);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("必须有目标");
        assert_eq!(
            bend_curves(&plan),
            vec![(0, Vec::new())],
            "空档弯音不许挂到任何音符上"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::PitchBend],
            MidiLossCount {
                read: 1,
                dropped: 1,
            },
            "read == attached + dropped（attached = 0）"
        );
        assert_eq!(
            plan.losses.aux_reasons[&MidiAuxEventKind::PitchBend],
            static_loss_reason(MidiAuxEventKind::PitchBend),
        );
    }

    /// ⭐ ADR-0014 D2 判据 c（复音不归属）：同通道同时发声 **2** 颗 ⇒ 弯音命中多颗
    /// ⇒ ⛔ 不复制给两颗、⛔ 不挑一颗，整条计入 `dropped`。
    #[test]
    fn a_pitch_bend_over_several_sounding_notes_is_dropped_not_copied() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = parsed(960, vec![note(60, 100, 0, 960), note(64, 100, 240, 720)]);
        parsed.pitch_bends = vec![bend(480, 100)]; // 两颗音符都在发声
        let bends = parsed.pitch_bends.len();
        let parsed = recording(parsed, MidiAuxEventKind::PitchBend, bends);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("必须有目标");
        assert_eq!(
            bend_curves(&plan),
            vec![(0, Vec::new()), (240, Vec::new())],
            "复音弯音一条都不许挂"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::PitchBend],
            MidiLossCount {
                read: 1,
                dropped: 1,
            },
            "read == attached + dropped（attached = 0）"
        );
    }

    /// ⭐ `#342`：报告把**每一类** aux 事件成对报出（read/dropped），有落点的四类
    /// 用**实际落地**的读数，没有落点的类仍 `dropped == read` 并给出精确原因。
    #[test]
    fn the_loss_report_pairs_read_and_dropped_for_every_aux_kind() {
        // 只有"没有内容"的工程才允许采纳单值 tempo/拍号 ⇒ 本夹具测那条分支。
        let project = content_free_project();
        let mut parsed = parsed(
            960,
            vec![
                ParsedNote {
                    track: 0,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    track: 0,
                    channel: 1,
                    key: 48,
                    velocity: 110,
                    start_tick: 0,
                    end_tick: 960,
                },
            ],
        );
        parsed.format = MidiFormat::Parallel;
        parsed.tracks = 3;
        for (kind, count) in [
            (MidiAuxEventKind::Tempo, 2),
            (MidiAuxEventKind::TimeSignature, 1),
            (MidiAuxEventKind::TrackName, 2),
            (MidiAuxEventKind::Marker, 2),
            (MidiAuxEventKind::ControlChange, 1),
            (MidiAuxEventKind::PitchBend, 1),
            (MidiAuxEventKind::UnknownMeta, 1),
        ] {
            parsed = recording(parsed, kind, count);
        }
        parsed.tempos = vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: Some(500_000),
                numerator: Some(3),
                denominator_pow2: Some(2),
            },
            MidiTempo {
                tick: 480,
                microseconds_per_quarter: Some(400_000),
                numerator: None,
                denominator_pow2: None,
            },
        ];
        parsed.track_names = vec![
            track_name(0, 0, Some("Lead")),
            track_name(1, 0, Some("Bass")),
        ];
        parsed.markers = vec![marker(0, Some("Intro")), marker(960, None)];
        parsed.end_tick = 1440;

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("必须有目标");
        let losses = &plan.losses;
        assert_eq!(losses.format, MidiFormat::Parallel);
        assert_eq!(losses.file_tracks, 3);
        assert_eq!(
            losses.track_partitions_dropped, 0,
            "多轨拆分开启时，没有未拆分的音轨分区 ⇒ 0 个分区丢失"
        );
        let plan_no_split = plan_midi_import(
            &project,
            &parsed,
            &MidiImportOptions {
                split_tracks: false,
                ..Default::default()
            },
        )
        .expect("必须有目标");
        assert_eq!(
            plan_no_split.losses.track_partitions_dropped, 2,
            "多轨拆分关闭时，3 条 MTrk 进 1 个目标 ⇒ 2 个分区丢失"
        );
        assert_eq!(losses.distinct_channels, 2, "音符里出现过 {{0,1}} 两个通道");
        assert_eq!(
            losses.note_channel_values,
            MidiLossCount {
                read: 2,
                dropped: 0,
            },
            "每颗音符一个通道值，已保存在 MidiNote.channel ⇒ dropped 为 0"
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

        // ---- 四类有落点：读到的条数 vs 真正落地的条数 ----
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::Tempo],
            MidiLossCount {
                read: 2,
                dropped: 0,
            },
            "tempo map（120→150）如实落地"
        );
        assert!(
            !losses.aux_reasons.contains_key(&MidiAuxEventKind::Tempo),
            "落地的类别不许编一条丢失原因"
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempoPoint {
                    tick: 480,
                    new_bpm: Some(bpm),
                    ..
                } if (*bpm - 150.0).abs() < 1e-4
            )),
            "tick 480 的 150 BPM 发射了 SetTempoPoint: {:?}",
            plan.ops
        );
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::TimeSignature],
            MidiLossCount {
                read: 1,
                dropped: 0,
            },
            "唯一条 3/4 @0 ⇒ 落进 project.time_signature"
        );
        assert!(
            !losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::TimeSignature),
            "落地的类别不许编一条丢失原因"
        );
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::TrackName],
            MidiLossCount {
                read: 2,
                dropped: 2,
            },
            "两个不同轨名不能都落进一个 TrackV3::name"
        );
        assert!(
            losses.aux_reasons[&MidiAuxEventKind::TrackName].contains("Lead")
                && losses.aux_reasons[&MidiAuxEventKind::TrackName].contains("Bass"),
            "轨名原因必须列出全部不同名字"
        );
        assert_eq!(
            losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 2,
                dropped: 1,
            },
            "1 条 marker 落成段落，非 UTF-8 那条落不下"
        );
        assert!(
            losses.aux_reasons[&MidiAuxEventKind::Marker].contains("not valid UTF-8"),
            "marker 原因必须精确到「非 UTF-8」: {:?}",
            losses.aux_reasons[&MidiAuxEventKind::Marker]
        );

        // ---- 本夹具里没有落点的三类：仍全损，且每类都有一条稳定原因（MCP 可逐字引用）----
        // ⚠ 弯音与控制器**可以**有落点（ADR-0014 / ADR-0015），但这份夹具只记了 aux
        // 读数、⛔ 没带 `pitch_bends` / `control_changes` 载荷 ⇒ 逐条仍是 `dropped`。
        for kind in [
            MidiAuxEventKind::ControlChange,
            MidiAuxEventKind::PitchBend,
            MidiAuxEventKind::UnknownMeta,
        ] {
            assert_eq!(
                losses.aux_events[&kind],
                MidiLossCount {
                    read: 1,
                    dropped: 1,
                },
                "{} 今天全损",
                kind.key()
            );
            assert_eq!(
                losses.aux_reasons[&kind],
                static_loss_reason(kind),
                "无落点的类别必须有原因（键 = {}）",
                kind.key()
            );
        }
        assert!(losses.is_consistent());
        assert!(
            losses.is_total_loss(),
            "无落点的类别必须仍全损（有落点的四类不参与这条判据）"
        );
        assert_eq!(losses.read_events(), 10, "夹具里真的 10 条 aux 事件");
        assert_eq!(
            losses.dropped_events(),
            2 + 1 + 3,
            "落地的是 tempo map (2) 与 3/4 (1)"
        );

        // ---- 后果：段落真的进了 ops（tick 已换算；非 UTF-8 只结束前一段） ----
        let sections: Vec<(&str, u64, u64)> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::SetSection { new_section, .. } => Some((
                    new_section.name.as_str(),
                    new_section.start_tick,
                    new_section.end_tick,
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            sections,
            vec![("Intro", 0_u64, 960_u64)],
            "只有 Intro 落成段落；非 UTF-8 的 marker 在 960 结束前一段"
        );
        assert!(plan.ops.iter().any(|op| matches!(
            op,
            Op::SetTimeSignature {
                new_time_signature: TimeSignature {
                    numerator: 3,
                    denominator: 4
                },
                ..
            }
        )));
        assert_eq!(plan.notes, 2);
        let note_channels: Vec<Option<u8>> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::AddNote { note, .. } => Some(note.channel),
                _ => None,
            })
            .collect();
        assert_eq!(note_channels, vec![Some(0), Some(1)], "音符通道被如实保留");
        assert_eq!(plan.lyrics, MidiLyricCounts::default());
    }

    /// ⭐ `#320`：歌词五字段只有一个 `Display`（CLI 与 GUI 回执共用）。
    #[test]
    fn lyric_counts_print_the_five_fields_once() {
        let counts = MidiLyricCounts {
            read: 2,
            attached: 1,
            dropped: 1,
            non_utf8: 0,
            empty: 0,
        };
        assert_eq!(
            counts.to_string(),
            "lyrics=2 attached=1 dropped=1 non-utf8=0 empty=0"
        );
        assert_eq!(
            MidiLyricCounts::default().to_string(),
            "lyrics=0 attached=0 dropped=0 non-utf8=0 empty=0"
        );
    }

    /// 造一份"24 类 / 29 条"的损失表（与 316 B 夹具同形），只测可见摘要文本。
    pub fn all_kinds_losses() -> MidiImportLosses {
        let mut aux_events = BTreeMap::new();
        for (index, kind) in MidiAuxEventKind::ALL.iter().copied().enumerate() {
            let read = if index == 0 { 6 } else { 1 };
            aux_events.insert(
                kind,
                MidiLossCount {
                    read,
                    dropped: read,
                },
            );
        }
        MidiImportLosses {
            format: MidiFormat::Parallel,
            file_tracks: 3,
            track_partitions_dropped: 2,
            distinct_channels: 2,
            note_channel_values: MidiLossCount {
                read: 5,
                dropped: 5,
            },
            note_off_velocity_values: MidiLossCount {
                read: 5,
                dropped: 5,
            },
            aux_events,
            aux_reasons: BTreeMap::new(),
        }
    }

    /// ⭐ `#320`：可见摘要保**既有六项语义**（导入丢失 / 类数 / 事件数 / 歌词未归属 /
    /// 通道 / 释放力度），并按需列出 `non_utf8` / `empty` 这两个丢失桶（⛔ 沉默）。
    /// 歌词全零时**逐字保持 `#319` 的旧文本**。
    #[test]
    fn the_visible_loss_summary_keeps_the_six_items_and_adds_nonzero_lyric_losses() {
        let losses = all_kinds_losses();
        assert_eq!(
            losses.loss_summary(MidiLyricCounts {
                read: 2,
                attached: 1,
                dropped: 1,
                non_utf8: 0,
                empty: 0,
            }),
            "⚠ 导入丢失：24 类/29 条；歌词未归属 1；通道/力度5/5",
            "316 B 夹具的可见摘要（`spare=7` 那一版）"
        );
        assert_eq!(
            losses.loss_summary(MidiLyricCounts {
                read: 3,
                attached: 2,
                dropped: 1,
                non_utf8: 1,
                empty: 2,
            }),
            "⚠ 导入丢失：24 类/29 条；歌词未归属1/非UTF8 1/空2",
            "非 UTF-8 / 空桶 > 0 时必须逐桶说出来（通道/力度留在 loss_detail）"
        );
        assert_eq!(
            losses.loss_summary(MidiLyricCounts {
                read: 2,
                attached: 2,
                dropped: 0,
                non_utf8: 0,
                empty: 0,
            }),
            "⚠ 导入丢失：24 类 / 29 条；通道值 5、释放力度 5",
            "无歌词丢失 ⇒ 旧文本逐字不变"
        );

        let no_aux = MidiImportLosses {
            format: MidiFormat::SingleTrack,
            file_tracks: 1,
            track_partitions_dropped: 0,
            distinct_channels: 1,
            note_channel_values: MidiLossCount {
                read: 5,
                dropped: 5,
            },
            note_off_velocity_values: MidiLossCount {
                read: 5,
                dropped: 5,
            },
            aux_events: BTreeMap::new(),
            aux_reasons: BTreeMap::new(),
        };
        assert_eq!(
            no_aux.loss_summary(MidiLyricCounts {
                read: 2,
                attached: 1,
                dropped: 1,
                non_utf8: 0,
                empty: 0,
            }),
            "⚠ 导入丢失：歌词未归属 1；通道/力度5/5"
        );
        assert_eq!(
            no_aux.loss_summary(MidiLyricCounts {
                read: 2,
                attached: 2,
                dropped: 0,
                non_utf8: 0,
                empty: 0,
            }),
            "⚠ 导入丢失：通道值 5、释放力度 5"
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
}

#[cfg(test)]
pub mod metadata_adoption_tests {
    use super::*;
    use crate::midi::{MidiAuxEventKind, MidiTempo, ParsedKeySignature, ParsedNote};

    /// 一个最小 `ParsedMidi`：2 颗音符、指定 ppq（元数据由调用方填）。
    fn two_notes(ppq: u16) -> ParsedMidi {
        let mut parsed = ParsedMidi {
            format: MidiFormat::SingleTrack,
            ppq,
            tracks: 1,
            aux_events: crate::midi::MidiAuxEventCounts::default(),
            tempos: Vec::<MidiTempo>::new(),
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: Vec::new(),
            instrument_names: Vec::new(),
            program_names: Vec::new(),
            markers: Vec::new(),
            end_tick: 0,
            notes: vec![
                ParsedNote {
                    track: 0,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    track: 0,
                    channel: 0,
                    key: 64,
                    velocity: 90,
                    start_tick: 480,
                    end_tick: 960,
                },
            ],
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };
        parsed.end_tick = 960;
        parsed
    }

    /// 从 ops 里读段落区间 `(name, start, end)`（模型 PPQ）。
    fn section_rows(plan: &MidiImportPlan) -> Vec<(String, u64, u64)> {
        plan.ops
            .iter()
            .filter_map(|op| match op {
                Op::SetSection { new_section, .. } => Some((
                    new_section.name.clone(),
                    new_section.start_tick,
                    new_section.end_tick,
                )),
                _ => None,
            })
            .collect()
    }

    fn marker(tick: u64, text: Option<&str>) -> ParsedMarker {
        ParsedMarker {
            tick,
            text: text.map(str::to_owned),
        }
    }

    fn track_name(track: usize, tick: u64, text: Option<&str>) -> ParsedTrackName {
        ParsedTrackName {
            track,
            tick,
            text: text.map(str::to_owned),
        }
    }

    fn text_meta(track: usize, tick: u64, text: Option<&str>) -> ParsedTrackName {
        // ADR-0013：`FF 04` / `FF 08` 与 `FF 03` 同形状（`ParsedTrackName`），复用夹具。
        track_name(track, tick, text)
    }

    fn recording(mut parsed: ParsedMidi, kind: MidiAuxEventKind, count: usize) -> ParsedMidi {
        for _ in 0..count {
            parsed.aux_events.record(kind);
        }
        parsed
    }

    /// 判据：**唯一条 tick 0** tempo 与拍号真的落进两个单标量；等值时**不**发空操作。
    #[test]
    fn a_single_tick_zero_tempo_and_meter_are_adopted() {
        let mut project = content_free_project();
        project.bpm = 120.0;
        project.time_signature = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Tempo, 1);
        parsed = recording(parsed, MidiAuxEventKind::TimeSignature, 1);
        parsed.tempos = vec![MidiTempo {
            tick: 0,
            microseconds_per_quarter: Some(600_000), // 100 BPM
            numerator: Some(7),
            denominator_pow2: Some(3), // 7/8
        }];

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempo { old_bpm, new_bpm } if *old_bpm == 120.0 && *new_bpm == 100.0
            )),
            "唯一 tick 0 tempo ⇒ SetTempo 120 ⇒ 100: {:?}",
            plan.ops
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTimeSignature { new_time_signature, .. }
                    if new_time_signature.numerator == 7 && new_time_signature.denominator == 8
            )),
            "唯一 tick 0 拍号 ⇒ SetTimeSignature 7/8: {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Tempo].dropped,
            0,
            "落地的 tempo 不是损失"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TimeSignature].dropped,
            0
        );
        assert!(plan.losses.aux_reasons.is_empty(), "没有损失就没有原因");

        // 等值：现值已经是文件值 ⇒ 不发 SetTempo / SetTimeSignature（⛔ 空操作）。
        let mut same = project.clone();
        same.bpm = 100.0;
        same.time_signature = TimeSignature {
            numerator: 7,
            denominator: 8,
        };
        let plan = plan_midi_import(&same, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetTempo { .. } | Op::SetTimeSignature { .. })),
            "现值已等于文件值 ⇒ 不发空操作: {:?}",
            plan.ops
        );
    }

    /// 判据（`#342` 修订）：**非空工程**的 `bpm` / `time_signature` 一字不动，
    /// 文件里的单值 tempo / 拍号作为**响亮损失**报出（原因点名值与 tick，并说明
    /// "工程已有内容，因此没有覆盖"）。
    #[test]
    fn a_non_empty_project_keeps_bpm_and_meter_and_names_the_values_it_did_not_adopt() {
        let mut project = yeban_model::samples::filled_project();
        project.bpm = 90.0;
        project.time_signature = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        assert!(!import_target_is_empty(&project), "filled 工程必须有内容");
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Tempo, 1);
        parsed = recording(parsed, MidiAuxEventKind::TimeSignature, 1);
        parsed.tempos = vec![MidiTempo {
            tick: 0,
            microseconds_per_quarter: Some(250_000), // 240 BPM
            numerator: Some(6),
            denominator_pow2: Some(3), // 6/8
        }];

        let plan = plan_midi_import(&project, &parsed, &MidiImportOptions::default())
            .expect("非空工程也能导入音符");
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetTempo { .. } | Op::SetTimeSignature { .. })),
            "非空工程 ⛔ 不许改全局 bpm / 拍号: {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Tempo],
            MidiLossCount {
                read: 1,
                dropped: 1,
            }
        );
        let tempo_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::Tempo];
        assert!(
            tempo_reason.contains("240.00 BPM @0")
                && tempo_reason.contains("already has content")
                && tempo_reason.contains("NOT written"),
            "tempo 原因必须点名值与 tick，并说明工程已有内容: {tempo_reason}"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TimeSignature],
            MidiLossCount {
                read: 1,
                dropped: 1,
            }
        );
        let meter_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::TimeSignature];
        assert!(
            meter_reason.contains("6/8 @0")
                && meter_reason.contains("already has content")
                && meter_reason.contains("NOT written"),
            "拍号原因必须点名值与 tick，并说明工程已有内容: {meter_reason}"
        );
    }

    /// 判据：`import_target_is_empty` 的**定义有牙** —— 四个维度各自都能把
    /// "没有内容"翻成"有内容"（否则内容门槛可能只在一种情况下生效）。
    #[test]
    fn the_content_free_definition_has_teeth() {
        assert!(
            import_target_is_empty(&content_free_project()),
            "夹具必须是'没有内容'"
        );
        assert!(
            !import_target_is_empty(&yeban_model::samples::filled_project()),
            "filled 工程有音符 ⇒ 不是'没有内容'"
        );

        // ① 一条 MIDI 音符。
        let mut notes = content_free_project();
        let clip_id = *notes.clip_pool.keys().next().expect("有一个片段");
        if let ClipContent::Midi { notes: map } =
            &mut notes.clip_pool.get_mut(&clip_id).unwrap().content
        {
            let note_id = EntityId::new();
            map.insert(note_id, MidiNote::new(note_id, 0, 60, 960));
        }
        assert!(!import_target_is_empty(&notes), "一条音符就算内容");

        // ② 一个音频片段。
        let mut audio = content_free_project();
        let audio_id = EntityId::new();
        audio.clip_pool.insert(
            audio_id,
            yeban_model::ClipPoolEntry {
                id: audio_id,
                name: "Audio".to_owned(),
                content: ClipContent::Audio {
                    asset: yeban_model::ids::AssetHash::of_bytes(b"content-free-test"),
                    gain_db: 0.0,
                },
            },
        );
        assert!(!import_target_is_empty(&audio), "音频片段就算内容");

        // ③ 一个段落。
        let mut sections = content_free_project();
        let section_id = EntityId::new();
        sections.sections.insert(
            section_id,
            SectionV3 {
                id: section_id,
                name: "Intro".to_owned(),
                start_tick: 0,
                end_tick: 960,
                color: None,
            },
        );
        assert!(!import_target_is_empty(&sections), "段落就算内容");

        // ④ 一个自动化点。
        let mut automation = content_free_project();
        let track_id = *automation
            .tracks
            .keys()
            .find(|id| **id != automation.master_bus_track_id)
            .expect("有一条非 master 音轨");
        let track = automation.tracks.get_mut(&track_id).expect("目标音轨");
        let target = *track
            .automation_lanes
            .keys()
            .next()
            .expect("演示夹具有一条自动化泳道");
        let point_id = EntityId::new();
        track
            .automation_lanes
            .get_mut(&target)
            .expect("泳道存在")
            .points
            .insert(
                point_id,
                yeban_model::AutomationPoint {
                    id: point_id,
                    tick: 0,
                    value: -6.0,
                    curve: yeban_model::CurveType::Linear,
                },
            );
        assert!(!import_target_is_empty(&automation), "自动化点就算内容");
    }

    /// 判据：**非空工程**遇到两条 tempo / 两条拍号必须响亮报丢并列出**两个值**，
    /// ⛔ 不许覆盖工程已有内容。工程内容必须**原样不动**。
    #[test]
    fn two_tempo_and_meter_changes_are_loud_losses_that_name_every_value() {
        let mut project = yeban_model::samples::filled_project();
        project.bpm = 120.0;
        project.time_signature = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Tempo, 2);
        parsed = recording(parsed, MidiAuxEventKind::TimeSignature, 2);
        parsed.tempos = vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: Some(600_000), // 100 BPM
                numerator: Some(5),
                denominator_pow2: Some(3), // 5/8
            },
            MidiTempo {
                tick: 960,
                microseconds_per_quarter: Some(400_000), // 150 BPM
                numerator: Some(3),
                denominator_pow2: Some(2), // 3/4
            },
        ];

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            !plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempo { .. }
                    | Op::SetTimeSignature { .. }
                    | Op::SetTempoPoint { .. }
                    | Op::SetTimeSignaturePoint { .. }
            )),
            "非空工程 ⛔ 不发 tempo/拍号操作: {:?}",
            plan.ops
        );
        let tempo = &plan.losses.aux_events[&MidiAuxEventKind::Tempo];
        assert_eq!((tempo.read, tempo.dropped), (2, 2), "两条都丢");
        let tempo_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::Tempo];
        assert!(
            tempo_reason.contains("100.00 BPM @0")
                && tempo_reason.contains("150.00 BPM @960")
                && tempo_reason.contains("already has content"),
            "tempo 原因必须同时点名两个值与 tick，并说明工程已有内容: {tempo_reason}"
        );
        let meter = &plan.losses.aux_events[&MidiAuxEventKind::TimeSignature];
        assert_eq!((meter.read, meter.dropped), (2, 2), "两条都丢");
        let meter_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::TimeSignature];
        assert!(
            meter_reason.contains("5/8 @0")
                && meter_reason.contains("3/4 @960")
                && meter_reason.contains("already has content"),
            "拍号原因必须同时点名两个值与 tick，并说明工程已有内容: {meter_reason}"
        );

        // 首条在 tick > 0 也一样：SMF 隐含 120 BPM / 4/4 ⇒ 仍是两个值。
        let mut late = two_notes(960);
        late = recording(late, MidiAuxEventKind::Tempo, 1);
        late = recording(late, MidiAuxEventKind::TimeSignature, 1);
        late.tempos = vec![MidiTempo {
            tick: 960,
            microseconds_per_quarter: Some(400_000),
            numerator: Some(3),
            denominator_pow2: Some(2),
        }];
        let plan =
            plan_midi_import(&project, &late, &MidiImportOptions::default()).expect("有目标");
        let tempo_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::Tempo];
        assert!(
            tempo_reason.contains("120.00 BPM @0 (SMF default")
                && tempo_reason.contains("150.00 BPM @960"),
            "首条 tempo 在 tick > 0 ⇒ 隐含 120 + 150: {tempo_reason}"
        );
        let meter_reason = &plan.losses.aux_reasons[&MidiAuxEventKind::TimeSignature];
        assert!(
            meter_reason.contains("4/4 @0 (SMF default") && meter_reason.contains("3/4 @960"),
            "首条拍号在 tick > 0 ⇒ 隐含 4/4 + 3/4: {meter_reason}"
        );
    }

    /// 判据：在没有内容的工程中，多条 tempo 与多条拍号如实落地进
    /// `project.bpm`/`tempo_map` 与 `project.time_signature`/`time_signature_map`，
    /// 损失为 0。首条在 tick > 0 时如实采用 SMF 默认值并在 tick > 0 建立点。
    #[test]
    fn multi_tempo_and_meter_changes_are_adopted_into_maps_with_zero_loss() {
        let mut project = content_free_project();
        project.bpm = 120.0;
        project.time_signature = TimeSignature {
            numerator: 4,
            denominator: 4,
        };
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Tempo, 2);
        parsed = recording(parsed, MidiAuxEventKind::TimeSignature, 2);
        parsed.tempos = vec![
            MidiTempo {
                tick: 0,
                microseconds_per_quarter: Some(600_000), // 100 BPM
                numerator: Some(5),
                denominator_pow2: Some(3), // 5/8
            },
            MidiTempo {
                tick: 960,
                microseconds_per_quarter: Some(400_000), // 150 BPM
                numerator: Some(3),
                denominator_pow2: Some(2), // 3/4
            },
        ];

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempo { old_bpm, new_bpm } if *old_bpm == 120.0 && *new_bpm == 100.0
            )),
            "tick 0 tempo 发射 SetTempo 120 => 100: {:?}",
            plan.ops
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempoPoint { tick: 960, new_bpm: Some(bpm), .. } if (*bpm - 150.0).abs() < 1e-4
            )),
            "tick 960 tempo 发射 SetTempoPoint: {:?}",
            plan.ops
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTimeSignature { new_time_signature, .. }
                    if new_time_signature.numerator == 5 && new_time_signature.denominator == 8
            )),
            "tick 0 拍号发射 SetTimeSignature 5/8: {:?}",
            plan.ops
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTimeSignaturePoint { tick: 960, new_time_signature: Some(ts), .. }
                    if ts.numerator == 3 && ts.denominator == 4
            )),
            "tick 960 拍号发射 SetTimeSignaturePoint 3/4: {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Tempo].dropped,
            0,
            "tempo map 如实采纳，0 丢失"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TimeSignature].dropped,
            0,
            "time signature map 如实采纳，0 丢失"
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::Tempo)
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::TimeSignature)
        );

        // 首条在 tick > 0：SMF 隐含 tick 0 为 120 BPM / 4/4。
        let mut late = two_notes(960);
        late = recording(late, MidiAuxEventKind::Tempo, 1);
        late = recording(late, MidiAuxEventKind::TimeSignature, 1);
        late.tempos = vec![MidiTempo {
            tick: 960,
            microseconds_per_quarter: Some(400_000), // 150 BPM
            numerator: Some(3),
            denominator_pow2: Some(2), // 3/4
        }];
        let plan =
            plan_midi_import(&project, &late, &MidiImportOptions::default()).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTempoPoint { tick: 960, new_bpm: Some(bpm), .. } if (*bpm - 150.0).abs() < 1e-4
            )),
            "late tempo 发射 SetTempoPoint 150 @960: {:?}",
            plan.ops
        );
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTimeSignaturePoint { tick: 960, new_time_signature: Some(ts), .. }
                    if ts.numerator == 3 && ts.denominator == 4
            )),
            "late meter 发射 SetTimeSignaturePoint 3/4 @960: {:?}",
            plan.ops
        );
        assert_eq!(plan.losses.aux_events[&MidiAuxEventKind::Tempo].dropped, 0);
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TimeSignature].dropped,
            0
        );
    }

    /// 判据：marker → `SectionV3` 区间，**逐 tick 逐名字**；区间两两不重叠
    /// （`SectionV3::validate` **不查重叠**，这里靠构造保证并断言）。
    #[test]
    fn markers_become_sections_with_exact_ticks_names_and_no_overlap() {
        let project = yeban_model::samples::filled_project();
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Marker, 3);
        parsed.markers = vec![
            marker(0, Some("Intro")),
            marker(960, Some("Sec \"1\"\n—")),
            marker(1920, Some("Outro")),
        ];
        parsed.end_tick = 2880;

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        let rows = section_rows(&plan);
        assert_eq!(
            rows,
            vec![
                ("Intro".to_owned(), 0, 960),
                ("Sec \"1\"\n—".to_owned(), 960, 1920),
                ("Outro".to_owned(), 1920, 2880),
            ],
            "一个 marker 一个段落；名字逐字节保留、区间到下一个 marker / end_tick"
        );
        for pair in rows.windows(2) {
            assert!(
                pair[0].2 <= pair[1].1,
                "相邻段落不许重叠: {:?} 与 {:?}",
                pair[0],
                pair[1]
            );
        }
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 3,
                dropped: 0,
            }
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::Marker)
        );
    }

    /// 判据：零长度（末尾 marker 与 `end_tick` 重合）与同 tick 顶替都必须**响亮**
    /// 报丢，⛔ 不许生成 `end <= start` 的 `SectionV3`（那会被模型拒绝）。
    #[test]
    fn zero_length_and_superseded_markers_are_loud_losses() {
        let project = yeban_model::samples::filled_project();

        // ① 末尾 marker @960 == end_tick ⇒ 零长度。
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Marker, 2);
        parsed.markers = vec![marker(0, Some("A")), marker(960, Some("B"))];
        parsed.end_tick = 960;
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert_eq!(section_rows(&plan), vec![("A".to_owned(), 0, 960)]);
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 2,
                dropped: 1,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::Marker];
        assert!(
            reason.contains("zero-length") && reason.contains("B"),
            "零长度必须点名 tick 与名字: {reason}"
        );

        // ② 同一 tick 两条 ⇒ 后一条胜出，前一条逐条报"顶替"。
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::Marker, 3);
        parsed.markers = vec![
            marker(0, Some("A")),
            marker(0, Some("B")),
            marker(960, Some("C")),
        ];
        parsed.end_tick = 1920;
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert_eq!(
            section_rows(&plan),
            vec![("B".to_owned(), 0, 960), ("C".to_owned(), 960, 1920)],
            "同一 tick 后一条胜出（MIDI 生效语义）"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 3,
                dropped: 1,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::Marker];
        assert!(
            reason.contains("superseded") && reason.contains("A"),
            "顶替必须点名: {reason}"
        );

        // ③ 低 PPQ 缩放把 1 tick 的区间塌成 0 长度 ⇒ 同样响亮报丢。
        let mut parsed = two_notes(1920);
        parsed = recording(parsed, MidiAuxEventKind::Marker, 2);
        parsed.markers = vec![marker(0, Some("A")), marker(1, Some("B"))];
        parsed.end_tick = 2;
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert_eq!(
            section_rows(&plan),
            vec![("B".to_owned(), 0, 1)],
            "0..1 缩放后塌成 0..0 ⇒ 只有 B 落成 0..1"
        );
        assert_eq!(plan.losses.aux_events[&MidiAuxEventKind::Marker].dropped, 1);
        assert!(
            plan.losses.aux_reasons[&MidiAuxEventKind::Marker].contains("zero-length"),
            "缩放塌陷也必须报零长度"
        );
    }

    /// 判据：没有 marker 的文件**不新增**段落（也**不**编一条损失）。
    #[test]
    fn a_file_without_markers_adds_no_sections() {
        let project = yeban_model::samples::filled_project();
        let parsed = two_notes(960);
        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(section_rows(&plan).is_empty(), "没有 marker ⇒ 没有新段落");
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::Marker],
            MidiLossCount {
                read: 0,
                dropped: 0,
            }
        );
        assert!(
            !plan
                .losses
                .aux_reasons
                .contains_key(&MidiAuxEventKind::Marker)
        );
    }

    /// 判据：轨名**去重后才采纳**，多个不同名字 / 空名 / 非 UTF-8 都不挑。
    #[test]
    fn track_names_are_adopted_only_when_one_distinct_name_remains() {
        let mut project = yeban_model::samples::filled_project();
        let track_id = *project
            .tracks
            .keys()
            .find(|id| **id != project.master_bus_track_id)
            .expect("有一条非 master 音轨");
        project.tracks.get_mut(&track_id).expect("目标音轨").name = "Original".to_owned();
        let options = MidiImportOptions {
            track_id: Some(track_id),
            clip_id: None,
            ..Default::default()
        };

        // ① 两个 MTrk 同名 ⇒ 去重成一个名字，落地（不算损失）。
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::TrackName, 2);
        parsed.track_names = vec![
            track_name(0, 0, Some("Piano")),
            track_name(1, 0, Some("Piano")),
        ];
        let plan = plan_midi_import(&project, &parsed, &options).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTrackName { new_name, old_name, .. }
                    if new_name == "Piano" && old_name == "Original"
            )),
            "同名去重后恰一个名字 ⇒ 采用: {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TrackName],
            MidiLossCount {
                read: 2,
                dropped: 0,
            },
            "同名去重 = 两条都落地"
        );

        // ② 两个不同名字 ⇒ ⛔ 不挑，逐条报丢。
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::TrackName, 2);
        parsed.track_names = vec![
            track_name(0, 0, Some("Lead")),
            track_name(1, 0, Some("Bass")),
        ];
        let plan = plan_midi_import(&project, &parsed, &options).expect("有目标");
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetTrackName { .. })),
            "两个不同名字 ⇒ ⛔ 改名"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TrackName],
            MidiLossCount {
                read: 2,
                dropped: 2,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::TrackName];
        assert!(
            reason.contains("Lead") && reason.contains("Bass"),
            "原因必须列出两个名字: {reason}"
        );

        // ③ 一个合法名字 + 空名 + 非 UTF-8 ⇒ 采用合法的那个，另两条报丢。
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::TrackName, 3);
        parsed.track_names = vec![
            track_name(0, 0, Some("Piano")),
            track_name(1, 0, Some("")),
            track_name(2, 0, None),
        ];
        let plan = plan_midi_import(&project, &parsed, &options).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetTrackName { new_name, .. } if new_name == "Piano"
            )),
            "唯一非空合法名字 ⇒ 采用"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::TrackName],
            MidiLossCount {
                read: 3,
                dropped: 2,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::TrackName];
        assert!(
            reason.contains("1 empty") && reason.contains("1 non-UTF-8"),
            "原因必须分别点名空名与非 UTF-8: {reason}"
        );
    }

    #[test]
    fn multi_track_midi_import_splits_subsequent_tracks_with_notes() {
        let project = yeban_model::samples::filled_project();
        let (dest_track, dest_clip) = destination(&project, &MidiImportOptions::default()).unwrap();

        let mut aux_events = crate::midi::MidiAuxEventCounts::default();
        aux_events.record(MidiAuxEventKind::TrackName);
        aux_events.record(MidiAuxEventKind::TrackName);

        let parsed = ParsedMidi {
            format: MidiFormat::Parallel,
            ppq: 960,
            tracks: 3,
            aux_events,
            tempos: Vec::new(),
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: vec![
                track_name(1, 0, Some("Lead Synth")),
                track_name(2, 0, Some("Sub Bass")),
            ],
            instrument_names: Vec::new(),
            program_names: Vec::new(),
            markers: Vec::new(),
            end_tick: 960,
            notes: vec![
                ParsedNote {
                    track: 1,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    track: 2,
                    channel: 1,
                    key: 36,
                    velocity: 90,
                    start_tick: 480,
                    end_tick: 960,
                },
            ],
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };

        let plan = plan_midi_import(&project, &parsed, &MidiImportOptions::default())
            .expect("多轨导入规划成功");

        // 验证产物：新轨、新片段、新摆放、新路由
        let mut new_track_id = None;
        let mut new_clip_id = None;
        let mut has_routing = false;

        for op in &plan.ops {
            match op {
                Op::AddTrack { track } => {
                    assert_eq!(track.name, "Sub Bass");
                    assert_eq!(track.kind, TrackKind::Midi);
                    new_track_id = Some(track.id);
                }
                Op::AddClip { clip } => {
                    assert_eq!(clip.name, "Sub Bass");
                    new_clip_id = Some(clip.id);
                }
                Op::AddClipPlacement {
                    track_id,
                    placement,
                } => {
                    assert_eq!(Some(*track_id), new_track_id);
                    assert_eq!(Some(placement.clip_id), new_clip_id);
                    assert_eq!(placement.start_tick, 0);
                    assert_eq!(placement.duration_ticks, 960);
                }
                Op::ConnectRouting { edge } => {
                    assert_eq!(Some(edge.source_node), new_track_id);
                    assert_eq!(edge.destination_node, project.master_bus_track_id);
                    assert_eq!(edge.kind, RoutingKind::TrackToBus);
                    has_routing = true;
                }
                _ => {}
            }
        }

        assert!(new_track_id.is_some(), "必须发射 Op::AddTrack");
        assert!(new_clip_id.is_some(), "必须发射 Op::AddClip");
        assert!(has_routing, "必须发射 Op::ConnectRouting");

        // 验证音符映射：第一轨落到目标片段，第二轨落到新创建片段
        let lead_note_op = plan
            .ops
            .iter()
            .find(|op| matches!(op, Op::AddNote { note, .. } if note.pitch == 60))
            .expect("Lead 音符操作必须存在");
        if let Op::AddNote {
            track_id, clip_id, ..
        } = lead_note_op
        {
            assert_eq!(*track_id, dest_track);
            assert_eq!(*clip_id, dest_clip);
        }

        let bass_note_op = plan
            .ops
            .iter()
            .find(|op| matches!(op, Op::AddNote { note, .. } if note.pitch == 36))
            .expect("Bass 音符操作必须存在");
        if let Op::AddNote {
            track_id, clip_id, ..
        } = bass_note_op
        {
            assert_eq!(Some(*track_id), new_track_id);
            assert_eq!(Some(*clip_id), new_clip_id);
        }

        assert_eq!(plan.notes, 2);
        assert_eq!(plan.losses.track_partitions_dropped, 0);
        assert!(plan.losses.is_consistent());
    }

    #[test]
    fn multi_track_midi_import_when_split_disabled() {
        let project = yeban_model::samples::filled_project();
        let (dest_track, dest_clip) = destination(&project, &MidiImportOptions::default()).unwrap();

        let mut aux_events = crate::midi::MidiAuxEventCounts::default();
        aux_events.record(MidiAuxEventKind::TrackName);
        aux_events.record(MidiAuxEventKind::TrackName);

        let parsed = ParsedMidi {
            format: MidiFormat::Parallel,
            ppq: 960,
            tracks: 3,
            aux_events,
            tempos: Vec::new(),
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: vec![
                track_name(1, 0, Some("Lead Synth")),
                track_name(2, 0, Some("Sub Bass")),
            ],
            instrument_names: Vec::new(),
            program_names: Vec::new(),
            markers: Vec::new(),
            end_tick: 960,
            notes: vec![
                ParsedNote {
                    track: 1,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    track: 2,
                    channel: 1,
                    key: 36,
                    velocity: 90,
                    start_tick: 480,
                    end_tick: 960,
                },
            ],
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };

        let options = MidiImportOptions {
            split_tracks: false,
            ..Default::default()
        };
        let plan = plan_midi_import(&project, &parsed, &options).expect("多轨导入规划成功");

        // 验证没有发射新建轨/片段/摆放/路由
        assert!(
            !plan.ops.iter().any(|op| matches!(
                op,
                Op::AddTrack { .. }
                    | Op::AddClip { .. }
                    | Op::AddClipPlacement { .. }
                    | Op::ConnectRouting { .. }
            )),
            "split_tracks = false 时不得发射新建轨操作"
        );

        // 两个音符都落入同一个目标片段
        for op in &plan.ops {
            if let Op::AddNote {
                track_id, clip_id, ..
            } = op
            {
                assert_eq!(*track_id, dest_track);
                assert_eq!(*clip_id, dest_clip);
            }
        }

        assert_eq!(plan.notes, 2);
        assert_eq!(plan.losses.track_partitions_dropped, 2);
        assert!(plan.losses.is_consistent());
    }

    #[test]
    fn multi_track_midi_import_fallback_track_name_when_no_meta() {
        let project = yeban_model::samples::filled_project();
        let parsed = ParsedMidi {
            format: MidiFormat::Parallel,
            ppq: 960,
            tracks: 2,
            aux_events: crate::midi::MidiAuxEventCounts::default(),
            tempos: Vec::new(),
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: Vec::new(), // 无轨名元事件
            instrument_names: Vec::new(),
            program_names: Vec::new(),
            markers: Vec::new(),
            end_tick: 480,
            notes: vec![
                ParsedNote {
                    track: 0,
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                },
                ParsedNote {
                    track: 1,
                    channel: 1,
                    key: 64,
                    velocity: 90,
                    start_tick: 0,
                    end_tick: 480,
                },
            ],
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };

        let plan = plan_midi_import(&project, &parsed, &MidiImportOptions::default()).unwrap();
        let add_track_op = plan
            .ops
            .iter()
            .find(|op| matches!(op, Op::AddTrack { .. }))
            .expect("第二轨必须创建新音轨");
        if let Op::AddTrack { track } = add_track_op {
            assert_eq!(track.name, "Track 2");
        }
    }

    /// ⭐ ADR-0013 D1/D2：拆分新建音轨的命名回退链
    /// `FF 03` → `FF 04` → `FF 08` → 机器名，以及"同类多个不同名字 = 歧义 ⇒ 下一级"。
    ///
    /// 控制臂：六个拆分轨各命中回退链的一个不同分支（乐器名次选 / 程序名末选 /
    /// 轨名首选 / 同类歧义降级 / 空与非 UTF-8 不参与 / 三级全歧义兜底），
    /// 实现若退化成"总取第一类"或"同类总取第一条"就会红。
    #[test]
    fn split_track_naming_follows_the_adr_0013_fallback_chain() {
        let project = yeban_model::samples::filled_project();
        let note = |track: usize| ParsedNote {
            track,
            channel: 0,
            key: 60,
            velocity: 100,
            start_tick: 0,
            end_tick: 480,
        };
        // aux 读数如实记录（dropped ≤ read 不变式）：3 条 FF 03 / 7 条 FF 04 / 4 条 FF 08。
        let mut aux_events = crate::midi::MidiAuxEventCounts::default();
        for (kind, count) in [
            (MidiAuxEventKind::TrackName, 3),
            (MidiAuxEventKind::InstrumentName, 7),
            (MidiAuxEventKind::ProgramName, 4),
        ] {
            for _ in 0..count {
                aux_events.record(kind);
            }
        }
        let parsed = ParsedMidi {
            format: MidiFormat::Parallel,
            ppq: 960,
            tracks: 7,
            aux_events,
            tempos: Vec::new(),
            key_signatures: Vec::new(),
            pitch_bends: Vec::new(),
            control_changes: Vec::new(),
            track_names: vec![
                text_meta(3, 0, Some("Lead")),
                text_meta(5, 0, Some("")), // 空名 ⇒ 无效（D2）
                text_meta(5, 48, None),    // 非 UTF-8 ⇒ 无效（D2）
            ],
            instrument_names: vec![
                text_meta(1, 0, Some("Violin I")),
                text_meta(3, 0, Some("Saw")),
                text_meta(4, 0, Some("A")),
                text_meta(4, 24, Some("B")), // 同类两个不同名字 ⇒ 歧义（D2）
                text_meta(5, 0, Some("Cello")),
                text_meta(6, 0, Some("A")),
                text_meta(6, 24, Some("B")), // 歧义
            ],
            program_names: vec![
                text_meta(2, 0, Some("Grand Piano")),
                text_meta(4, 0, Some("Brass")),
                text_meta(6, 0, Some("P1")),
                text_meta(6, 24, Some("P2")), // 歧义 ⇒ 机器兜底名
            ],
            markers: Vec::new(),
            end_tick: 480,
            notes: (0..7).map(note).collect(),
            lyrics: Vec::new(),
            non_utf8_lyrics: 0,
            empty_lyrics: 0,
        };

        let plan = plan_midi_import(&project, &parsed, &MidiImportOptions::default()).unwrap();
        let names: Vec<String> = plan
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::AddTrack { track } => Some(track.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "Violin I".to_owned(),    // D1：无 FF 03，FF 04 次选生效
                "Grand Piano".to_owned(), // D1：无 FF 03/04，FF 08 末选生效
                "Lead".to_owned(),        // D1：FF 03 优先于 FF 04
                "Brass".to_owned(),       // D2：FF 04 歧义 ⇒ 降到 FF 08
                "Cello".to_owned(),       // D2：空 / 非 UTF-8 的 FF 03 不参与匹配
                "Track 7".to_owned(),     // 三级全歧义 ⇒ 机器兜底名（track_index + 1）
            ],
            "拆分新轨命名必须按 D1/D2 逐 MTrk 下标 1..6 发射"
        );
    }

    /// 判据（`#350`）：**唯一条 tick 0** 调号在空工程中落进 `project.key_signature`；等值时不发空操作。
    #[test]
    fn a_single_tick_zero_key_signature_is_adopted_in_empty_project() {
        let mut project = content_free_project();
        project.key_signature = None;
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::KeySignature, 1);
        parsed.key_signatures = vec![ParsedKeySignature {
            tick: 0,
            sharps_flats: 2, // D Major (2 升)
            is_minor: false,
        }];

        let plan =
            plan_midi_import(&project, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            plan.ops.iter().any(|op| matches!(
                op,
                Op::SetKeySignature {
                    old_key_signature: None,
                    new_key_signature: Some(KeySignature {
                        sharps_flats: 2,
                        is_minor: false,
                    }),
                }
            )),
            "唯一 tick 0 调号 ⇒ SetKeySignature None -> Some(2, false): {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::KeySignature].dropped,
            0,
            "落地的 key_signature 不是损失"
        );
        assert!(plan.losses.aux_reasons.is_empty(), "没有损失就没有原因");

        // 等值：现值已经是文件值 ⇒ 不发 SetKeySignature（空操作）。
        let mut same = project.clone();
        same.key_signature = Some(KeySignature {
            sharps_flats: 2,
            is_minor: false,
        });
        let plan = plan_midi_import(&same, &parsed, &MidiImportOptions::default()).expect("有目标");
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetKeySignature { .. })),
            "现值已等于文件值 ⇒ 不发空操作: {:?}",
            plan.ops
        );
    }

    /// 判据（`#350`）：非空工程保留原调号，文件里的调号作为响亮损失报出。
    #[test]
    fn a_non_empty_project_keeps_key_signature_and_reports_loss() {
        let mut project = yeban_model::samples::filled_project();
        project.key_signature = Some(KeySignature {
            sharps_flats: -1,
            is_minor: false,
        });
        assert!(!import_target_is_empty(&project), "filled 工程必须有内容");
        let mut parsed = two_notes(960);
        parsed = recording(parsed, MidiAuxEventKind::KeySignature, 1);
        parsed.key_signatures = vec![ParsedKeySignature {
            tick: 0,
            sharps_flats: 3,
            is_minor: true, // F# minor
        }];

        let plan = plan_midi_import(&project, &parsed, &MidiImportOptions::default())
            .expect("非空工程也能导入音符");
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetKeySignature { .. })),
            "非空工程不得改全局调号: {:?}",
            plan.ops
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::KeySignature],
            MidiLossCount {
                read: 1,
                dropped: 1,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::KeySignature];
        assert!(
            reason.contains("3 sf, minor=true @0")
                && reason.contains("already has content")
                && reason.contains("NOT written"),
            "调号原因必须点名值与 tick，并说明工程已有内容: {reason}"
        );
    }

    /// 判据（`#350`）：调号 map（多条调号或 tick > 0）与超界调号作为响亮损失报出。
    #[test]
    fn key_signature_map_and_out_of_range_are_loud_losses() {
        let project = content_free_project();

        // 1. 调号 map（两条不同调号）
        let mut parsed_map = two_notes(960);
        parsed_map = recording(parsed_map, MidiAuxEventKind::KeySignature, 2);
        parsed_map.key_signatures = vec![
            ParsedKeySignature {
                tick: 0,
                sharps_flats: 0,
                is_minor: false,
            },
            ParsedKeySignature {
                tick: 960,
                sharps_flats: 1,
                is_minor: false,
            },
        ];
        let plan = plan_midi_import(&project, &parsed_map, &MidiImportOptions::default()).unwrap();
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetKeySignature { .. })),
            "调号 map 不得采纳"
        );
        assert_eq!(
            plan.losses.aux_events[&MidiAuxEventKind::KeySignature],
            MidiLossCount {
                read: 2,
                dropped: 2,
            }
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::KeySignature];
        assert!(
            reason.contains("key-signature-map-not-representable")
                && reason.contains("0 sf (minor=false) @0")
                && reason.contains("1 sf (minor=false) @960"),
            "map 原因必须列出全部调号事件: {reason}"
        );

        // 2. 超界调号（sharps_flats = 8）
        let mut parsed_oor = two_notes(960);
        parsed_oor = recording(parsed_oor, MidiAuxEventKind::KeySignature, 1);
        parsed_oor.key_signatures = vec![ParsedKeySignature {
            tick: 0,
            sharps_flats: 8,
            is_minor: false,
        }];
        let plan = plan_midi_import(&project, &parsed_oor, &MidiImportOptions::default()).unwrap();
        assert!(
            !plan
                .ops
                .iter()
                .any(|op| matches!(op, Op::SetKeySignature { .. })),
            "超界调号不得采纳"
        );
        let reason = &plan.losses.aux_reasons[&MidiAuxEventKind::KeySignature];
        assert!(
            reason.contains("key-signature-out-of-model-range")
                && reason.contains("sharps_flats=8"),
            "超界原因必须点名: {reason}"
        );
    }
}
