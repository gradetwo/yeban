//! 标准 MIDI 文件 (SMF) 0/1 导出与回读 [ARCH-FMT-001 §5.5]。
//!
//! > **[ARCH-FMT-001 §5.5]** 基于 `midly` 库实现零堆分配的高速 SMF 序列化导出,
//! > 支持 Tempo Map 拍速标记、拍号变更与多通道独立分轨导出。
//!
//! ## 用别人的编码器 + 自己的解码器
//!
//! 导出走 `midly`（上游实现, 已核验 API 与 feature 门: `Smf::write_std` 需要
//! `midly` 的 `std` feature, 而根清单用 `default-features = false` 登记,
//! 因此本 crate 显式打开 `std` —— 见 `Cargo.toml` 注释）。
//!
//! **回读走 `midly` 的解析器, 但结构断言走本 crate 自己的字节级检查**:
//! [`track_chunks`] 直接按 SMF 的 chunk 头解析 `MThd`/`MTrk`（返回 [`TrackChunk`]）,
//! [`crate::vlq`] 是
//! 一份独立的 VLQ 解码器。因此判据是"两个独立实现互相钉住", 而不是"`midly`
//! 读自己写的东西" —— 后者对"库的编码与规范不符"完全无感。
//!
//! ## 关键事实（已核验, 见 notes §1）
//!
//! - SMF 头是 `MThd` + 大端 `u32` 长度(6) + 大端 `u16` 格式 + 大端 `u16` 轨道数
//!   + 大端 `u16` 时间分度 (Metrical 时的 PPQ);
//! - delta-time 与 meta 长度用 VLQ, 单值上限 `0x0FFF_FFFF` (28 位);
//! - `midly` **不会**自动追加 `EndOfTrack`（已核对 `smf.rs` 的 `write_raw` 只写
//!   传入的事件）; 因此本模块显式追加 `FF 2F 00`;
//! - `NoteOn` 且力度为 0 在约定上等价于 `NoteOff`。
//!
//! ## 边界（这次没有证明什么）
//!
//! - 拍号变更 (`TimeSignature`)、调号、滑音 (`SlideConfig`)、弯音曲线、
//!   连击 (`ratchet`)、触发概率 (< 1.0)、歌词与音素**不导出**;
//!   登记在 notes 的 `pending`。`micro_timing_ticks` 会并入起始 tick。
//! - 只导出 Metrical (PPQ) 时间分度; SMPTE 时间码文件在回读时被拒绝
//!   ([`MidiError::UnsupportedTimecode`])。
//! - 回读是**严格**的: 未闭合的音符与未匹配的 `NoteOff` 都是错误, 而不是被忽略。

use std::str::FromStr;

use midly::num::{u4, u7, u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, Smf, Timing, Track, TrackEvent, TrackEventKind,
};
use yeban_model::{EntityId, MidiNote};

/// 默认 PPQ（夜半的整数时钟基准 [MODEL-AST-001]）。
pub const DEFAULT_PPQ: u16 = 960;

/// 一个待导出的轨道。
///
/// 只有 `PartialEq`（没有 `Eq`）: 内含的 `yeban_model::MidiNote` 带 `probability: Option<f32>`。
#[derive(Clone, Debug, PartialEq)]
pub struct MidiExportTrack {
    /// 轨道名（写进 `TrackName` meta; 空字符串则不写该 meta）。
    pub name: String,
    /// MIDI 通道 0..=15。
    pub channel: u8,
    /// 音符（来自 `yeban_model::MidiNote`, 单一事实源）。
    pub notes: Vec<MidiNote>,
}

/// tempo map 里的一条记录。
///
/// 一条记录最多携带**两个** SMF 元事件: 一个 `Tempo` (`FF 51 03`) 与一个
/// `TimeSignature` (`FF 58 04`)。两个字段各自可以为 `None`
/// ⇒ 记录因此能如实表达真文件的三种形状: "只有 tempo"、"只有拍号"、"两者都有"。
/// "只有拍号"**不必**再凭空合成一条 `500000 µs` 的 tempo
/// （凭空合成是被修掉的缺陷; 见 `docs/ledger/integration-rulings-notes.md` 的 R5）。
///
/// ⚠️ `numerator` 与 `denominator_pow2` 是**一对**：只给其中一个时 `to_smf_bytes`
/// 回 [`MidiError::HalfTimeSignature`]（既不写事件、也不许凭空补另一半，更不许静默
/// 丢掉整条记录）。两个都 `None` 才是"这条记录不带拍号"。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiTempo {
    /// 绝对 tick。
    pub tick: u64,
    /// 每四分音符的微秒数 (`mpqn`)。`None` = 这条记录**不写** `Tempo` 事件
    /// （只写拍号）。
    pub microseconds_per_quarter: Option<u32>,
    /// 拍号分子 (拍/小节)。`None` 表示不写拍号事件。
    pub numerator: Option<u8>,
    /// 拍号分母的以 2 为底的幂 (4 = 四分音符)。
    pub denominator_pow2: Option<u8>,
}

impl MidiTempo {
    /// 由 BPM 换算 `mpqn`（四舍五入到微秒）。
    ///
    /// `mpqn = 60_000_000 / bpm`。走 `f64` 再四舍五入: BPM 是用户输入的十进制数,
    /// 用整数除法会把 120 BPM 变成 499999 微秒（真值 500000）。
    #[must_use]
    pub fn from_bpm(bpm: f64) -> Self {
        let mpqn = if bpm > 0.0 && bpm.is_finite() {
            (60_000_000.0 / bpm).round().clamp(1.0, 16_777_215.0) as u32
        } else {
            500_000
        };
        Self {
            tick: 0,
            microseconds_per_quarter: Some(mpqn),
            numerator: None,
            denominator_pow2: None,
        }
    }

    /// 由 BPM 与拍号构造。
    #[must_use]
    pub fn with_time_signature(bpm: f64, numerator: u8, denominator_pow2: u8) -> Self {
        Self {
            numerator: Some(numerator),
            denominator_pow2: Some(denominator_pow2),
            ..Self::from_bpm(bpm)
        }
    }
}

/// 导出格式: SMF 0 (单轨) 或 SMF 1 (多轨并行)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MidiFormat {
    /// `MThd` 格式 0: 整个文件只有一条 `MTrk`, 所有通道塞在一起。
    SingleTrack,
    /// `MThd` 格式 1: 多条 `MTrk` 同时播放, 第 0 条是 conductor (tempo map)。
    Parallel,
}

impl MidiFormat {
    /// 写进 `MThd` 的格式号。
    #[must_use]
    pub const fn number(self) -> u16 {
        match self {
            Self::SingleTrack => 0,
            Self::Parallel => 1,
        }
    }

    /// 由格式号反解。
    #[must_use]
    pub const fn from_number(number: u16) -> Option<Self> {
        match number {
            0 => Some(Self::SingleTrack),
            1 => Some(Self::Parallel),
            _ => None,
        }
    }
}

/// 一次导出的完整输入。
#[derive(Clone, Debug, PartialEq)]
pub struct MidiExport {
    /// 格式。
    pub format: MidiFormat,
    /// 时间分度 (PPQ)。
    pub ppq: u16,
    /// Tempo map。每条记录带 `tick` 与可选的事件对 (tempo / 拍号)。
    ///
    /// 写出前按**内容**规范化排序（不是按本 `Vec` 的输入顺序）⇒ 导出字节只由内容
    /// 决定; 同一 tick 上"只有拍号"的记录排在"带 tempo"的记录之前。
    pub tempos: Vec<MidiTempo>,
    /// 轨道。
    pub tracks: Vec<MidiExportTrack>,
}

/// 导出失败的原因。
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MidiError {
    /// PPQ 为 0 或超过 15 位上限。
    InvalidPpq(u16),
    /// 通道号超过 15。
    ChannelOutOfRange(u8),
    /// 音高超过 127。
    PitchOutOfRange(u8),
    /// 力度超过 127。
    VelocityOutOfRange(u8),
    /// 时值为 0。
    ZeroDuration {
        /// 出问题的音符身份。
        note: EntityId,
    },
    /// 某个 delta-time 超过 VLQ 的 28 位上限。
    DeltaOverflow {
        /// 绝对 tick。
        tick: u64,
        /// 实际 delta。
        delta: u64,
    },
    /// 没有可写的轨道 (格式 1 至少需要一条)。
    NoTracks,
    /// `midly` 编码失败。
    Encode(String),
    /// `midly` 解析失败。
    Decode(String),
    /// 回读遇到 SMPTE 时间码 (`Timing::Timecode`), 本切片不支持。
    UnsupportedTimecode,
    /// 回读遇到非 0/1 的 SMF 格式号。
    UnsupportedFormat(u16),
    /// 回读发现未闭合的音符 (缺少 `NoteOff`)。
    UnclosedNote {
        /// `NoteOn` 所在的绝对 tick。
        start_tick: u64,
        /// 音高。
        key: u8,
    },
    /// 回读发现没有对应 `NoteOn` 的 `NoteOff`。
    UnmatchedNoteOff {
        /// 绝对 tick。
        tick: u64,
        /// 音高。
        key: u8,
    },
    /// tempo map 里的一条记录**只给了一半**拍号（分子与分母的以 2 为底的幂只有一个）。
    ///
    /// `FF 58 04` 的两个字段必须**成对**：只给一个时既写不出这个元事件，也**不许凭空补**
    /// 另一半（R5 修掉的正是"凭空合成"）。修之前这条记录被**静默丢掉**（整个 tick 上的
    /// 内容一起消失，调用方收到的却是 `Ok`）⇒ 现在明确拒绝。
    HalfTimeSignature {
        /// 出问题的 tick。
        tick: u64,
        /// 分子；`None` = 没给。
        numerator: Option<u8>,
        /// 分母的以 2 为底的幂；`None` = 没给。
        denominator_pow2: Option<u8>,
    },
}

impl core::fmt::Display for MidiError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPpq(ppq) => write!(f, "PPQ 非法: {ppq}"),
            Self::ChannelOutOfRange(channel) => write!(f, "通道号越界: {channel}"),
            Self::PitchOutOfRange(pitch) => write!(f, "音高越界: {pitch}"),
            Self::VelocityOutOfRange(velocity) => write!(f, "力度越界: {velocity}"),
            Self::ZeroDuration { note } => write!(f, "音符 {note} 的时值为 0"),
            Self::DeltaOverflow { tick, delta } => {
                write!(f, "tick {tick} 处的 delta {delta} 超过 VLQ 的 28 位上限")
            }
            Self::NoTracks => f.write_str("没有可导出的轨道"),
            Self::Encode(message) => write!(f, "SMF 编码失败: {message}"),
            Self::Decode(message) => write!(f, "SMF 解析失败: {message}"),
            Self::UnsupportedTimecode => f.write_str("不支持 SMPTE 时间码分度"),
            Self::UnsupportedFormat(number) => write!(f, "不支持的 SMF 格式号: {number}"),
            Self::UnclosedNote { start_tick, key } => {
                write!(f, "tick {start_tick} 的音符 (key {key}) 没有 NoteOff")
            }
            Self::UnmatchedNoteOff { tick, key } => {
                write!(f, "tick {tick} 的 NoteOff (key {key}) 没有对应 NoteOn")
            }
            Self::HalfTimeSignature {
                tick,
                numerator,
                denominator_pow2,
            } => write!(
                f,
                "tick {tick} 的拍号只给了一半: numerator = {numerator:?}, \
                 denominator_pow2 = {denominator_pow2:?}"
            ),
        }
    }
}

impl std::error::Error for MidiError {}

/// `to_smf_bytes` 内部的一个**事件组**: 一条 [`MidiTempo`] 折成的
/// "可选 `Tempo` + 可选拍号"。
///
/// 它同时是排序键 (`Ord` 按字段顺序): 先 `tick`, 再 `Option<u32>` 的 `None < Some`
/// ⇒ "只有拍号"的组排在"带 tempo"的组之前。这正是回读规则的逆: 回读把拍号挂到
/// **紧邻的前一条**未配拍号的 tempo 上, 因此没有前驱 tempo 的拍号必然落成
/// "只有拍号"的记录, 而带 tempo 的组里两个事件相邻 ⇒ 配对原样回来。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct TempoGroup {
    /// 绝对 tick。
    tick: u64,
    /// 折进 u24 的 `mpqn`; `None` = 这一组不写 `Tempo` 事件。
    microseconds_per_quarter: Option<u32>,
    /// 拍号 `(numerator, denominator_pow2)`; `None` = 这一组不写拍号事件。
    signature: Option<(u8, u8)>,
}

impl TempoGroup {
    /// 由一条 [`MidiTempo`] 折成一组 (两个事件值都取 u8/u24 的可表达范围)。
    ///
    /// # Errors
    ///
    /// 拍号**只给了一半** ⇒ [`MidiError::HalfTimeSignature`]。`FF 58 04` 的两个字段必须
    /// 成对：只给一个时既写不出拍号事件，也不许凭空补另一半（R5 修掉的正是"凭空合成"），
    /// 而静默丢掉整条记录会让调用方以为它写出去了。
    fn of(tempo: &MidiTempo) -> Result<Self, MidiError> {
        let signature = match (tempo.numerator, tempo.denominator_pow2) {
            (Some(numerator), Some(denominator_pow2)) => Some((numerator, denominator_pow2)),
            (None, None) => None,
            (numerator, denominator_pow2) => {
                return Err(MidiError::HalfTimeSignature {
                    tick: tempo.tick,
                    numerator,
                    denominator_pow2,
                });
            }
        };
        Ok(Self {
            tick: tempo.tick,
            microseconds_per_quarter: tempo
                .microseconds_per_quarter
                .map(|mpqn| mpqn.min(0x00FF_FFFF)),
            signature,
        })
    }

    /// 这一组是否会写出至少一个事件 (两个都是 `None` 的组写不出任何字节)。
    fn carries_an_event(&self) -> bool {
        self.microseconds_per_quarter.is_some() || self.signature.is_some()
    }
}

/// 内部规范化事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RawEvent {
    NoteOn {
        tick: u64,
        channel: u8,
        key: u8,
        velocity: u8,
    },
    NoteOff {
        tick: u64,
        channel: u8,
        key: u8,
    },
    Tempo {
        tick: u64,
        /// tempo map 里的**组号**: 同一条 [`MidiTempo`] 的两个事件共享它 ⇒
        /// 它们相邻落盘, 回读时才能配对回同一条记录。
        group: u32,
        microseconds_per_quarter: u32,
    },
    TimeSignature {
        tick: u64,
        /// 见 [`RawEvent::Tempo`] 的 `group`。
        group: u32,
        numerator: u8,
        denominator_pow2: u8,
    },
}

impl RawEvent {
    fn tick(self) -> u64 {
        match self {
            Self::NoteOn { tick, .. }
            | Self::NoteOff { tick, .. }
            | Self::Tempo { tick, .. }
            | Self::TimeSignature { tick, .. } => tick,
        }
    }

    /// 同一 tick 内的全序键: `(rank, 组, 组内序, 值)`。
    ///
    /// 必须给出**全序**, 否则同一 tick 的多个事件会按输入顺序落到文件里,
    /// 让"同一工程两次导出字节不同"。
    ///
    /// tempo map 的两类事件同属 `rank 2`, 且以 `(组, 组内序)` 排序:
    /// 一条记录的 `Tempo` 是组内序 `0`、它的拍号是组内序 `1`
    /// ⇒ 两者**相邻**, 而回读时"拍号挂到紧邻的前一条未配拍号的 tempo"
    /// 恰好把它配回同一条记录。组号由内容决定 (见 `to_smf_bytes`), 因此字节仍然
    /// 只由内容决定。
    fn tie_break(self) -> (u8, u32, u8, u32) {
        match self {
            Self::NoteOff { channel, key, .. } => (0, u32::from(channel), 0, u32::from(key)),
            Self::NoteOn {
                channel,
                key,
                velocity,
                ..
            } => (
                1,
                u32::from(channel),
                0,
                (u32::from(key) << 8) | u32::from(velocity),
            ),
            Self::Tempo {
                group,
                microseconds_per_quarter,
                ..
            } => (2, group, 0, microseconds_per_quarter),
            Self::TimeSignature {
                group,
                numerator,
                denominator_pow2,
                ..
            } => (
                2,
                group,
                1,
                (u32::from(numerator) << 8) | u32::from(denominator_pow2),
            ),
        }
    }
}

/// 把一个音符的起始 tick 规范化 (把微时值并入, 并钳到 0)。
fn effective_start(note: &MidiNote) -> u64 {
    let offset = note.micro_timing_ticks.unwrap_or(0);
    if offset >= 0 {
        note.start_tick.saturating_add(offset as u64)
    } else {
        note.start_tick.saturating_sub(offset.unsigned_abs() as u64)
    }
}

/// 校验一个音符可以导出。
fn validate_note(note: &MidiNote, channel: u8) -> Result<(), MidiError> {
    if channel > 15 {
        return Err(MidiError::ChannelOutOfRange(channel));
    }
    if note.pitch > 127 {
        return Err(MidiError::PitchOutOfRange(note.pitch));
    }
    if note.velocity > 127 {
        return Err(MidiError::VelocityOutOfRange(note.velocity));
    }
    if note.duration_ticks == 0 {
        return Err(MidiError::ZeroDuration { note: note.id });
    }
    Ok(())
}

/// 把一条轨道的音符展开成规范化事件。
fn track_events(track: &MidiExportTrack) -> Result<Vec<RawEvent>, MidiError> {
    let mut events = Vec::with_capacity(track.notes.len() * 2);
    for note in &track.notes {
        validate_note(note, track.channel)?;
        let start = effective_start(note);
        let end = start.saturating_add(note.duration_ticks);
        events.push(RawEvent::NoteOn {
            tick: start,
            channel: track.channel,
            key: note.pitch,
            velocity: note.velocity,
        });
        events.push(RawEvent::NoteOff {
            tick: end,
            channel: track.channel,
            key: note.pitch,
        });
    }
    sort_events(&mut events);
    Ok(events)
}

/// 按 `(tick, tie_break)` 全序排序 —— 导出字节因此只由内容决定。
fn sort_events(events: &mut [RawEvent]) {
    events.sort_by_key(|event| (event.tick(), event.tie_break()));
}

/// 把绝对 tick 序列折成 delta, 并按 VLQ 上限校验。
fn deltas(ticks: impl Iterator<Item = u64>) -> Result<Vec<u32>, MidiError> {
    let mut out = Vec::new();
    let mut previous = 0u64;
    for tick in ticks {
        let delta = tick.saturating_sub(previous);
        if delta > u64::from(crate::vlq::VLQ_MAX) {
            return Err(MidiError::DeltaOverflow { tick, delta });
        }
        out.push(delta as u32);
        previous = tick;
    }
    Ok(out)
}

impl MidiExport {
    /// 编码为 SMF 字节。
    ///
    /// # Errors
    ///
    /// 见 [`MidiError`]。
    pub fn to_smf_bytes(&self) -> Result<Vec<u8>, MidiError> {
        // `MThd` 的时间分度是 **15 位**字段, 而 `midly` 的 `u15::new` 是**掩码**
        // (`raw & 0x7FFF`)、不是拒绝 ⇒ 超过上界的 ppq 会被**静默回绕**成另一个值。
        // 实测 (修之前, 单位 = 无量纲的 ppq 读数): 请求 `0x8000` 落盘成 `0x0000`,
        // 请求 `0x83C0` 落盘成 `0x03C0` (= 960), 请求 `0xFFFF` 落盘成 `0x7FFF`。
        // 那正是 `InvalidPpq` 的文档已经点名、却只实现了一半的情形。
        if self.ppq == 0 || self.ppq > 0x7FFF {
            return Err(MidiError::InvalidPpq(self.ppq));
        }
        if self.tracks.is_empty() {
            return Err(MidiError::NoTracks);
        }

        // tempo map 独立于轨道, 先规范化。
        //
        // 步骤 ①: 把每条 `MidiTempo` 折成一个**组** [`TempoGroup`], 丢掉两个事件都
        // 没有的空组; 步骤 ②: 组按**内容**升序排列 (不是按 `self.tempos` 的
        // 输入顺序) ⇒ 字节只由内容决定; 步骤 ③: 组号就是排序后的下标, 同一组的两个
        // 事件共享组号 ⇒ 它们相邻落盘 (见 `RawEvent::tie_break`)。
        let mut groups: Vec<TempoGroup> = self
            .tempos
            .iter()
            .map(TempoGroup::of)
            .collect::<Result<_, _>>()?;
        groups.retain(TempoGroup::carries_an_event);
        groups.sort_unstable();

        let mut tempo_events: Vec<RawEvent> = Vec::new();
        for (group, entry) in groups.into_iter().enumerate() {
            let group = u32::try_from(group).expect("组的条数远小于 u32 的上限");
            if let Some(microseconds_per_quarter) = entry.microseconds_per_quarter {
                tempo_events.push(RawEvent::Tempo {
                    tick: entry.tick,
                    group,
                    microseconds_per_quarter,
                });
            }
            if let Some((numerator, denominator_pow2)) = entry.signature {
                tempo_events.push(RawEvent::TimeSignature {
                    tick: entry.tick,
                    group,
                    numerator,
                    denominator_pow2,
                });
            }
        }
        sort_events(&mut tempo_events);

        // 每条输出轨道 = 一串规范化事件。
        let mut lanes: Vec<(Vec<u8>, Vec<RawEvent>)> = Vec::new();
        match self.format {
            MidiFormat::Parallel => {
                lanes.push((b"Yeban Conductor".to_vec(), tempo_events));
                for track in &self.tracks {
                    lanes.push((track.name.as_bytes().to_vec(), track_events(track)?));
                }
            }
            MidiFormat::SingleTrack => {
                // 格式 0: 全部塞进一条 MTrk; 轨道名只在恰好一条输入轨道时保留。
                let name = if self.tracks.len() == 1 {
                    self.tracks[0].name.as_bytes().to_vec()
                } else {
                    Vec::new()
                };
                let mut events = tempo_events;
                for track in &self.tracks {
                    events.extend(track_events(track)?);
                }
                sort_events(&mut events);
                lanes.push((name, events));
            }
        }

        let header = Header::new(
            match self.format {
                MidiFormat::SingleTrack => Format::SingleTrack,
                MidiFormat::Parallel => Format::Parallel,
            },
            Timing::Metrical(u15::new(self.ppq)),
        );

        // 名字的字节必须活得比 `tracks` 久, 因此用 `unzip` 把名字从车道里**移出**来,
        // 而不是 clone 一份（`names` 的借用要活到 `tracks` 用完为止）。
        let (names, event_lanes): (Vec<Vec<u8>>, Vec<Vec<RawEvent>>) = lanes.into_iter().unzip();
        let mut tracks: Vec<Track<'_>> = Vec::with_capacity(event_lanes.len());
        for (index, events) in event_lanes.iter().enumerate() {
            let delta_list = deltas(events.iter().map(|event| event.tick()))?;
            let mut lane: Track<'_> = Vec::with_capacity(events.len() + 2);
            if !names[index].is_empty() {
                lane.push(TrackEvent {
                    delta: u28::new(0),
                    kind: TrackEventKind::Meta(MetaMessage::TrackName(&names[index])),
                });
            }
            for (event, delta) in events.iter().zip(&delta_list) {
                let kind = match *event {
                    RawEvent::NoteOn {
                        channel,
                        key,
                        velocity,
                        ..
                    } => TrackEventKind::Midi {
                        channel: u4::new(channel),
                        message: MidiMessage::NoteOn {
                            key: u7::new(key),
                            vel: u7::new(velocity),
                        },
                    },
                    RawEvent::NoteOff { channel, key, .. } => TrackEventKind::Midi {
                        channel: u4::new(channel),
                        message: MidiMessage::NoteOff {
                            key: u7::new(key),
                            vel: u7::new(0),
                        },
                    },
                    RawEvent::Tempo {
                        microseconds_per_quarter,
                        ..
                    } => {
                        TrackEventKind::Meta(MetaMessage::Tempo(u24::new(microseconds_per_quarter)))
                    }
                    RawEvent::TimeSignature {
                        numerator,
                        denominator_pow2,
                        ..
                    } => TrackEventKind::Meta(MetaMessage::TimeSignature(
                        numerator,
                        denominator_pow2,
                        24,
                        8,
                    )),
                };
                lane.push(TrackEvent {
                    delta: u28::new(*delta),
                    kind,
                });
            }
            // `midly` 不会自动追加 `EndOfTrack`（已核对上游 `write_raw`）, 必须自己写。
            lane.push(TrackEvent {
                delta: u28::new(0),
                kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
            });
            tracks.push(lane);
        }

        let smf = Smf { header, tracks };
        let mut out = Vec::new();
        smf.write_std(&mut out)
            .map_err(|error| MidiError::Encode(error.to_string()))?;
        Ok(out)
    }
}

/// 回读得到的一个音符。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParsedNote {
    /// MIDI 通道。
    pub channel: u8,
    /// 音高。
    pub key: u8,
    /// 力度。
    pub velocity: u8,
    /// 起始绝对 tick。
    pub start_tick: u64,
    /// 结束绝对 tick。
    pub end_tick: u64,
}

impl ParsedNote {
    /// 时值 (tick)。
    #[must_use]
    pub const fn duration_ticks(self) -> u64 {
        self.end_tick - self.start_tick
    }

    /// 与导出侧比较时的规范化键 `(channel, key, velocity, start_tick, duration)`。
    #[must_use]
    pub const fn key(self) -> (u8, u8, u8, u64, u64) {
        (
            self.channel,
            self.key,
            self.velocity,
            self.start_tick,
            self.duration_ticks(),
        )
    }
}

/// 回读结果。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParsedMidi {
    /// 格式。
    pub format: MidiFormat,
    /// 时间分度 (PPQ)。
    pub ppq: u16,
    /// tempo map（按 tick 升序, 按文件里出现的顺序）。
    pub tempos: Vec<MidiTempo>,
    /// 全部音符, 按 `(start_tick, channel, key, velocity)` 排序。
    pub notes: Vec<ParsedNote>,
}

/// SMF 里的一个 chunk: fourcc 与它在文件里的负载范围。
///
/// 用具名结构而不是 `([u8; 4], Range<usize>)` 元组: 后者会触发
/// `clippy::type_complexity`, 而且 `chunk.0` / `chunk.1` 在调用点无法自解释。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackChunk {
    /// chunk 标识 (`MThd` / `MTrk` / 其他)。
    pub fourcc: [u8; 4],
    /// 负载在文件字节里的范围。
    pub payload: std::ops::Range<usize>,
}

impl TrackChunk {
    /// 负载长度。
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// `true` 表示空负载。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }
}

/// 按 SMF 的 chunk 头把一个文件拆成 [`TrackChunk`] 列表。
///
/// 这是**本 crate 自己的**结构解析（大端 `u32` 长度）, 用来独立核验 `midly`
/// 写出的 chunk 布局, 而不是相信 `midly` 自己的解析器。
///
/// # Errors
///
/// 声明长度超出实际字节时返回 [`MidiError::Decode`]。
pub fn track_chunks(bytes: &[u8]) -> Result<Vec<TrackChunk>, MidiError> {
    let mut cursor = 0usize;
    let mut out = Vec::new();
    while cursor + 8 <= bytes.len() {
        let fourcc: [u8; 4] = bytes[cursor..cursor + 4].try_into().expect("4 字节");
        let len = u32::from_be_bytes([
            bytes[cursor + 4],
            bytes[cursor + 5],
            bytes[cursor + 6],
            bytes[cursor + 7],
        ]) as usize;
        let start = cursor + 8;
        if start + len > bytes.len() {
            return Err(MidiError::Decode(format!(
                "chunk {} 声明 {len} 字节, 但文件只剩 {} 字节",
                String::from_utf8_lossy(&fourcc),
                bytes.len() - start
            )));
        }
        out.push(TrackChunk {
            fourcc,
            payload: start..start + len,
        });
        cursor = start + len;
    }
    Ok(out)
}

/// 解析 SMF 字节。
///
/// SMF 1.0 要求 `MThd` 的 Metrical 时间分度是**正数** ⇒ 分度为 0 的头部明确
/// [`MidiError::InvalidPpq`]，而不是回出一个 `ppq == 0` 的读数（下游按 ppq 换算 tick
/// 就会除零，而本 crate 的导出侧本来就拒绝写出 0 ⇒ 只拒一半是不对称的）。
///
/// # Errors
///
/// 见 [`MidiError`]。
pub fn parse_smf(bytes: &[u8]) -> Result<ParsedMidi, MidiError> {
    let smf = Smf::parse(bytes).map_err(|error| MidiError::Decode(error.to_string()))?;
    let format = match smf.header.format {
        Format::SingleTrack => MidiFormat::SingleTrack,
        Format::Parallel => MidiFormat::Parallel,
        Format::Sequential => {
            return Err(MidiError::UnsupportedFormat(2));
        }
    };
    let ppq = match smf.header.timing {
        Timing::Metrical(division) => division.as_int(),
        Timing::Timecode(..) => return Err(MidiError::UnsupportedTimecode),
    };
    if ppq == 0 {
        return Err(MidiError::InvalidPpq(ppq));
    }

    let mut notes = Vec::new();
    let mut tempos = Vec::new();
    for track in &smf.tracks {
        let mut tick = 0u64;
        // (channel, key) -> (start_tick, velocity); 用 Vec 保序, 便于"后开先关"匹配。
        let mut open: Vec<(u8, u8, u64, u8)> = Vec::new();
        for event in track {
            tick += u64::from(event.delta.as_int());
            match event.kind {
                TrackEventKind::Midi { channel, message } => {
                    let channel = channel.as_int();
                    match message {
                        MidiMessage::NoteOn { key, vel } => {
                            let key = key.as_int();
                            let vel = vel.as_int();
                            if vel == 0 {
                                close_note(&mut open, &mut notes, channel, key, tick)?;
                            } else {
                                open.push((channel, key, tick, vel));
                            }
                        }
                        MidiMessage::NoteOff { key, .. } => {
                            close_note(&mut open, &mut notes, channel, key.as_int(), tick)?;
                        }
                        _ => {}
                    }
                }
                TrackEventKind::Meta(MetaMessage::Tempo(mpqn)) => tempos.push(MidiTempo {
                    tick,
                    microseconds_per_quarter: Some(mpqn.as_int()),
                    numerator: None,
                    denominator_pow2: None,
                }),
                TrackEventKind::Meta(MetaMessage::TimeSignature(
                    numerator,
                    denominator_pow2,
                    _clocks,
                    _notes,
                )) => {
                    // 配对按**出现顺序**, 且只看**紧邻的前一条**记录:
                    // 拍号挂到紧邻的那条同 tick、有 tempo、还没配拍号的记录上。
                    // 没有这样的前驱 ⇒ 这条拍号**如实**落成"只有拍号"的记录,
                    // **不**凭空合成 tempo (R5 修掉的正是合成)。
                    let attached = match tempos.last_mut() {
                        Some(last)
                            if last.tick == tick
                                && last.microseconds_per_quarter.is_some()
                                && last.numerator.is_none() =>
                        {
                            last.numerator = Some(numerator);
                            last.denominator_pow2 = Some(denominator_pow2);
                            true
                        }
                        _ => false,
                    };
                    if !attached {
                        tempos.push(MidiTempo {
                            tick,
                            microseconds_per_quarter: None,
                            numerator: Some(numerator),
                            denominator_pow2: Some(denominator_pow2),
                        });
                    }
                }
                _ => {}
            }
        }
        if let Some(&(_, key, start_tick, _)) = open.first() {
            return Err(MidiError::UnclosedNote { start_tick, key });
        }
    }

    notes.sort_by_key(|note| note.key());
    Ok(ParsedMidi {
        format,
        ppq,
        tempos,
        notes,
    })
}

/// 关掉一个已开启的音符 (后开先关)。
fn close_note(
    open: &mut Vec<(u8, u8, u64, u8)>,
    notes: &mut Vec<ParsedNote>,
    channel: u8,
    key: u8,
    tick: u64,
) -> Result<(), MidiError> {
    let Some(index) = open
        .iter()
        .rposition(|&(open_channel, open_key, _, _)| open_channel == channel && open_key == key)
    else {
        return Err(MidiError::UnmatchedNoteOff { tick, key });
    };
    let (_, _, start_tick, velocity) = open.remove(index);
    notes.push(ParsedNote {
        channel,
        key,
        velocity,
        start_tick,
        end_tick: tick,
    });
    Ok(())
}

/// 由模型层的 `MidiNote` 集合构造一条导出轨道。
///
/// 通道按 `channel` 参数统一给定; 分通道导出请调用方按通道拆成多条轨道。
#[must_use]
pub fn track_from_notes(
    name: impl Into<String>,
    channel: u8,
    notes: &[MidiNote],
) -> MidiExportTrack {
    MidiExportTrack {
        name: name.into(),
        channel,
        notes: notes.to_vec(),
    }
}

/// 解析一个 26 字符 ULID 文本（测试与工具用）。
///
/// # Errors
///
/// 非法 ULID 文本。
pub fn entity_id(text: &str) -> Result<EntityId, MidiError> {
    EntityId::from_str(text).map_err(|error| MidiError::Decode(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(start_tick: u64, pitch: u8, duration_ticks: u64, velocity: u8) -> MidiNote {
        MidiNote {
            velocity,
            ..MidiNote::new(EntityId::new(), start_tick, pitch, duration_ticks)
        }
    }

    fn export(format: MidiFormat, tracks: Vec<MidiExportTrack>) -> MidiExport {
        MidiExport {
            format,
            ppq: DEFAULT_PPQ,
            tempos: vec![MidiTempo::with_time_signature(120.0, 4, 2)],
            tracks,
        }
    }

    /// 判据 1: 导出的字节能被解析回**同一个音符集合** (格式 1, 多通道)。
    #[test]
    fn parallel_export_round_trips_the_same_note_set() {
        let source = export(
            MidiFormat::Parallel,
            vec![
                track_from_notes(
                    "Bass",
                    1,
                    &[
                        note(0, 36, 480, 100),
                        note(480, 38, 240, 90),
                        note(960, 41, 960, 110),
                    ],
                ),
                track_from_notes(
                    "Lead",
                    2,
                    &[note(240, 72, 480, 80), note(960, 74, 120, 127)],
                ),
            ],
        );
        let bytes = source.to_smf_bytes().expect("导出");
        let parsed = parse_smf(&bytes).expect("回读");

        assert_eq!(parsed.format, MidiFormat::Parallel);
        assert_eq!(parsed.ppq, DEFAULT_PPQ);
        assert_eq!(parsed.notes.len(), 5, "五颗音符一颗不少");

        let mut expected: Vec<(u8, u8, u8, u64, u64)> = vec![
            (1, 36, 100, 0, 480),
            (1, 38, 90, 480, 240),
            (1, 41, 110, 960, 960),
            (2, 72, 80, 240, 480),
            (2, 74, 127, 960, 120),
        ];
        expected.sort_unstable();
        let mut actual: Vec<(u8, u8, u8, u64, u64)> =
            parsed.notes.iter().map(|note| note.key()).collect();
        actual.sort_unstable();
        assert_eq!(actual, expected);
    }

    /// 判据 2: 格式 0 把所有通道压进一条 `MTrk`, 且音符集合不变。
    #[test]
    fn single_track_export_merges_channels_into_one_chunk() {
        let source = export(
            MidiFormat::SingleTrack,
            vec![
                track_from_notes("A", 0, &[note(0, 60, 480, 64)]),
                track_from_notes("B", 3, &[note(0, 64, 480, 64), note(480, 67, 480, 64)]),
            ],
        );
        let bytes = source.to_smf_bytes().expect("导出");
        assert_eq!(bytes[8..10], [0x00, 0x00], "MThd 格式号必须是 0");
        assert_eq!(bytes[10..12], [0x00, 0x01], "格式 0 只能有一条轨道");

        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(chunks.len(), 2, "MThd + 一条 MTrk");
        assert_eq!(&chunks[0].fourcc, b"MThd");
        // `payload` 是**文件内**的字节范围（不是相对 chunk 起点的范围）:
        // fourcc 0..4, 大端长度 4..8, 负载 8..14。
        assert_eq!(chunks[0].payload, 8..14, "MThd 负载在文件里的范围");
        assert_eq!(chunks[0].len(), 6);
        assert!(!chunks[0].is_empty());
        assert_eq!(&chunks[1].fourcc, b"MTrk");

        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.format, MidiFormat::SingleTrack);
        assert_eq!(parsed.notes.len(), 3);
        let mut actual: Vec<(u8, u8, u8, u64, u64)> =
            parsed.notes.iter().map(|note| note.key()).collect();
        actual.sort_unstable();
        assert_eq!(
            actual,
            vec![
                (0, 60, 64, 0, 480),
                (3, 64, 64, 0, 480),
                (3, 67, 64, 480, 480)
            ]
        );
    }

    /// 判据 3 (**VLQ 边界**, 用本 crate 独立的 VLQ 解码器核验 `midly` 的字节):
    /// tick = `0x0FFF_FFFF` 必须编码成 `FF FF FF 7F`; PPQ 与格式号按大端写在
    /// `MThd` 里。
    #[test]
    fn vlq_boundary_tick_is_encoded_as_four_bytes() {
        let boundary = u64::from(crate::vlq::VLQ_MAX);
        let source = MidiExport {
            format: MidiFormat::SingleTrack,
            ppq: DEFAULT_PPQ,
            tempos: Vec::new(),
            // 轨道名为空 -> 不写 TrackName meta, 于是 MTrk 的第一个事件就是这颗音符。
            tracks: vec![track_from_notes("", 0, &[note(boundary, 60, 120, 64)])],
        };
        let bytes = source.to_smf_bytes().expect("导出");

        // 独立核验 MThd: 大端格式号 + 轨道数 + 时间分度。
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(&chunks[0].fourcc, b"MThd");
        assert_eq!(
            u16::from_be_bytes([bytes[8], bytes[9]]),
            0,
            "格式号 0 (大端)"
        );
        assert_eq!(
            u16::from_be_bytes([bytes[10], bytes[11]]),
            1,
            "轨道数 1 (大端)"
        );
        assert_eq!(
            u16::from_be_bytes([bytes[12], bytes[13]]),
            DEFAULT_PPQ,
            "时间分度 = 960 PPQ (大端)"
        );

        // MTrk 负载从 delta 开始: 4 字节 VLQ + `90 3C 40` (NoteOn ch0 key60 vel64)。
        let payload = &bytes[chunks[1].payload.clone()];
        assert_eq!(
            &payload[0..4],
            &[0xFF, 0xFF, 0xFF, 0x7F],
            "0x0FFF_FFFF 的 VLQ 必须是 FF FF FF 7F"
        );
        let mut cursor = 0;
        assert_eq!(
            crate::vlq::decode(payload, &mut cursor),
            Some(0x0FFF_FFFF),
            "本 crate 的独立 VLQ 解码器必须读出同一个值"
        );
        assert_eq!(cursor, 4);
        assert_eq!(
            &payload[4..7],
            &[0x90, 0x3C, 0x40],
            "NoteOn ch0 key60 vel64"
        );
        assert_eq!(
            payload[payload.len() - 3..],
            [0xFF, 0x2F, 0x00],
            "每条轨道必须以 EndOfTrack 收尾"
        );

        // 而且真的能读回同一颗音符。
        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.notes.len(), 1);
        assert_eq!(parsed.notes[0].start_tick, boundary);
        assert_eq!(parsed.notes[0].duration_ticks(), 120);
    }

    /// 判据 4: 超出 28 位的 delta 被**拒绝**, 而不是被截断。
    #[test]
    fn delta_beyond_vlq_limit_is_rejected() {
        let too_far = u64::from(crate::vlq::VLQ_MAX) + 1;
        let source = export(
            MidiFormat::SingleTrack,
            vec![track_from_notes("", 0, &[note(too_far, 60, 120, 64)])],
        );
        assert_eq!(
            source.to_smf_bytes(),
            Err(MidiError::DeltaOverflow {
                tick: too_far,
                delta: too_far
            })
        );
    }

    /// 判据 5: tempo 事件按 tick 写出并可读回 (`mpqn` 与拍号)。
    #[test]
    fn tempo_map_round_trips() {
        let source = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![
                MidiTempo::with_time_signature(120.0, 4, 2),
                MidiTempo {
                    tick: 960,
                    ..MidiTempo::from_bpm(90.0)
                },
            ],
            tracks: vec![track_from_notes("T", 0, &[note(0, 60, 960, 64)])],
        };
        let bytes = source.to_smf_bytes().expect("导出");
        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.tempos.len(), 2);
        assert_eq!(
            parsed.tempos[0].microseconds_per_quarter,
            Some(500_000),
            "120 BPM"
        );
        assert_eq!(parsed.tempos[0].numerator, Some(4));
        assert_eq!(parsed.tempos[0].denominator_pow2, Some(2));
        assert_eq!(parsed.tempos[1].tick, 960);
        assert_eq!(
            parsed.tempos[1].microseconds_per_quarter,
            Some(666_667),
            "90 BPM"
        );
        // conductor 轨道存在 -> 格式 1 有两条 MTrk
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(chunks.len(), 3, "MThd + conductor + 一条音符轨");
    }

    /// 判据 5b (R5): 同一 tick 上"只有拍号"的记录与"tempo + 拍号"的记录
    /// 往返后**逐条不变**, 且不凭空多出 tempo 事件。
    ///
    /// 台账 `docs/ledger/integration-rulings-notes.md` 的 R5 (MIDI tempo map
    /// 往返不保真) 修复后加牙: 修好之前, `parse_smf` 会给"只有拍号"的记录合成一条
    /// `500000 µs` 的 tempo, 而 `to_smf_bytes` 再把它写进文件 ⇒ 事件集合多一条。
    #[test]
    fn a_signature_only_record_survives_the_round_trip_without_a_synthesised_tempo() {
        // tick 0: 只有拍号 3/8; tick 0 之后 960: 120 BPM + 4/4。
        let source = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: None,
                    numerator: Some(3),
                    denominator_pow2: Some(3),
                },
                MidiTempo {
                    tick: 960,
                    ..MidiTempo::with_time_signature(120.0, 4, 2)
                },
            ],
            tracks: vec![track_from_notes("T", 0, &[note(0, 60, 960, 64)])],
        };
        let bytes = source.to_smf_bytes().expect("导出");
        let parsed = parse_smf(&bytes).expect("回读");

        assert_eq!(
            parsed.tempos, source.tempos,
            "只有拍号的记录必须原样回来, 且不得多出合成的 tempo"
        );
        assert_eq!(parsed.tempos.len(), 2, "两条记录 ⇒ 两个事件组");
        assert_eq!(
            parsed.tempos[0].microseconds_per_quarter, None,
            "源文件里 tick 0 没有 tempo 事件 ⇒ 回读也不许有"
        );
        assert_eq!(parsed.tempos[1].microseconds_per_quarter, Some(500_000));
    }

    /// 判据 5c (R5): 同一 tick 上**两条** tempo + **两条**拍号, 源顺序交错 ⇒
    /// 往返后配对不换人, 事件集合逐条不变。
    #[test]
    fn two_tempos_and_two_signatures_on_one_tick_keep_their_pairing() {
        // 输出顺序由内容决定: 带 tempo 的组按 (tempo, 拍号) 升序相邻写出。
        // 这里两条记录各自带自己的拍号, 因此回读必须配回**同一条**记录。
        let source = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: Some(833_333),
                    numerator: Some(3),
                    denominator_pow2: Some(2),
                },
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: Some(500_000),
                    numerator: Some(4),
                    denominator_pow2: Some(2),
                },
            ],
            tracks: vec![track_from_notes("T", 0, &[note(0, 60, 960, 64)])],
        };
        let parsed = parse_smf(&source.to_smf_bytes().expect("导出")).expect("回读");
        // 记录**多重集**相等（`tick, tempo, 拍号` 三者成组比较 ⇒ 配对也在此断言里）。
        let key = |tempo: &MidiTempo| {
            (
                tempo.tick,
                tempo.microseconds_per_quarter,
                tempo.numerator,
                tempo.denominator_pow2,
            )
        };
        let mut want: Vec<_> = source.tempos.iter().map(key).collect();
        want.sort_unstable();
        let mut got: Vec<_> = parsed.tempos.iter().map(key).collect();
        got.sort_unstable();
        assert_eq!(
            got, want,
            "同一 tick 的两条记录必须配回各自的原值 (不许换人)"
        );
    }

    /// 判据 6: 同一导出两次调用产出**逐字节相同**的文件。
    #[test]
    fn export_is_byte_deterministic() {
        let source = export(
            MidiFormat::Parallel,
            vec![
                track_from_notes("X", 0, &[note(0, 60, 240, 64), note(240, 62, 240, 64)]),
                track_from_notes("Y", 5, &[note(120, 48, 480, 100)]),
            ],
        );
        let first = source.to_smf_bytes().expect("导出");
        let second = source.to_smf_bytes().expect("导出");
        assert_eq!(first, second);
    }

    /// 判据 7: 非法输入 (音高/力度/通道/时值/PPQ) 被明确拒绝。
    #[test]
    fn invalid_notes_are_rejected() {
        let mut bad_pitch = note(0, 60, 120, 64);
        bad_pitch.pitch = 200;
        assert_eq!(
            export(
                MidiFormat::SingleTrack,
                vec![track_from_notes("", 0, &[bad_pitch])]
            )
            .to_smf_bytes(),
            Err(MidiError::PitchOutOfRange(200))
        );

        let mut bad_velocity = note(0, 60, 120, 64);
        bad_velocity.velocity = 128;
        assert_eq!(
            export(
                MidiFormat::SingleTrack,
                vec![track_from_notes("", 0, &[bad_velocity])]
            )
            .to_smf_bytes(),
            Err(MidiError::VelocityOutOfRange(128))
        );

        let bad_channel = track_from_notes("", 16, &[note(0, 60, 120, 64)]);
        assert_eq!(
            export(MidiFormat::SingleTrack, vec![bad_channel]).to_smf_bytes(),
            Err(MidiError::ChannelOutOfRange(16))
        );

        let zero = note(0, 60, 0, 64);
        // `zero` 会被移入 `&[zero]` 这个临时数组, 因此先把 id 取出来（`EntityId` 是 `Copy`）。
        let zero_id = zero.id;
        match export(
            MidiFormat::SingleTrack,
            vec![track_from_notes("", 0, &[zero])],
        )
        .to_smf_bytes()
        {
            Err(MidiError::ZeroDuration { note }) => assert_eq!(note, zero_id),
            other => panic!("期望 ZeroDuration, 得到 {other:?}"),
        }

        let no_ppq = MidiExport {
            ppq: 0,
            ..export(MidiFormat::SingleTrack, vec![track_from_notes("", 0, &[])])
        };
        assert_eq!(no_ppq.to_smf_bytes(), Err(MidiError::InvalidPpq(0)));

        let no_tracks = MidiExport {
            tracks: Vec::new(),
            ..export(MidiFormat::SingleTrack, vec![track_from_notes("", 0, &[])])
        };
        assert_eq!(no_tracks.to_smf_bytes(), Err(MidiError::NoTracks));
    }

    /// 判据 8: 回读是严格的 —— 未闭合音符与孤立 `NoteOff` 都报错。
    #[test]
    fn parser_rejects_unbalanced_notes() {
        // 手工拼一个只有 NoteOn 的文件 (格式 0, 96 PPQ)。
        let mut payload = vec![0x00, 0x90, 0x3C, 0x40];
        payload.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&96u16.to_be_bytes());
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&payload);
        match parse_smf(&bytes) {
            Err(MidiError::UnclosedNote { start_tick, key }) => {
                assert_eq!(start_tick, 0);
                assert_eq!(key, 60);
            }
            other => panic!("期望 UnclosedNote, 得到 {other:?}"),
        }

        // 只有 NoteOff。
        let mut payload = vec![0x00, 0x80, 0x3C, 0x40];
        payload.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&96u16.to_be_bytes());
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&payload);
        assert!(matches!(
            parse_smf(&bytes),
            Err(MidiError::UnmatchedNoteOff { .. })
        ));
    }

    /// 判据 9: `NoteOn` 力度 0 按约定等价于 `NoteOff`。
    #[test]
    fn note_on_with_zero_velocity_closes_the_note() {
        let mut payload = vec![0x00, 0x90, 0x3C, 0x40]; // NoteOn
        payload.extend_from_slice(&[0x60, 0x90, 0x3C, 0x00]); // +96 tick, NoteOn vel 0
        payload.extend_from_slice(&[0x00, 0xFF, 0x2F, 0x00]);
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&0u16.to_be_bytes());
        bytes.extend_from_slice(&1u16.to_be_bytes());
        bytes.extend_from_slice(&96u16.to_be_bytes());
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(&payload);

        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.ppq, 96);
        assert_eq!(parsed.notes.len(), 1);
        assert_eq!(parsed.notes[0].start_tick, 0);
        assert_eq!(parsed.notes[0].end_tick, 96);
    }

    /// 判据 10: `micro_timing_ticks` 并入起始 tick, 且不会下溢到负数。
    #[test]
    fn micro_timing_shifts_the_start_tick_without_underflow() {
        let mut shifted = note(100, 60, 120, 64);
        shifted.micro_timing_ticks = Some(-30);
        assert_eq!(effective_start(&shifted), 70);
        let mut forward = note(100, 60, 120, 64);
        forward.micro_timing_ticks = Some(30);
        assert_eq!(effective_start(&forward), 130);
        let mut underflow = note(10, 60, 120, 64);
        underflow.micro_timing_ticks = Some(-100);
        assert_eq!(effective_start(&underflow), 0, "必须饱和到 0, 不能下溢");

        let bytes = export(
            MidiFormat::SingleTrack,
            vec![track_from_notes("", 0, &[shifted])],
        )
        .to_smf_bytes()
        .expect("导出");
        assert_eq!(parse_smf(&bytes).expect("回读").notes[0].start_tick, 70);
    }

    /// 判据 11: 同一 tick 上先关后开 —— 相邻同音高音符不会被并成一个长音。
    #[test]
    fn note_off_precedes_note_on_at_the_same_tick() {
        let bytes = export(
            MidiFormat::SingleTrack,
            vec![track_from_notes(
                "",
                0,
                &[note(0, 60, 480, 64), note(480, 60, 480, 64)],
            )],
        )
        .to_smf_bytes()
        .expect("导出");
        let parsed = parse_smf(&bytes).expect("回读");
        assert_eq!(parsed.notes.len(), 2, "必须是两颗音符, 不是一颗长音");
        assert_eq!(parsed.notes[0].end_tick, 480);
        assert_eq!(parsed.notes[1].start_tick, 480);
    }

    /// 判据 12: `from_bpm` 的取整与钳制。
    #[test]
    fn bpm_conversion_is_correct_and_clamped() {
        assert_eq!(
            MidiTempo::from_bpm(120.0).microseconds_per_quarter,
            Some(500_000)
        );
        assert_eq!(
            MidiTempo::from_bpm(90.0).microseconds_per_quarter,
            Some(666_667)
        );
        assert_eq!(
            MidiTempo::from_bpm(60.0).microseconds_per_quarter,
            Some(1_000_000)
        );
        assert_eq!(
            MidiTempo::from_bpm(0.0).microseconds_per_quarter,
            Some(500_000),
            "非法 BPM 回退到 120"
        );
        assert!(MidiTempo::from_bpm(1.0e12).microseconds_per_quarter <= Some(0x00FF_FFFF));
    }

    /// 判据 (类别⑦ delta 累加溢出): 起点 + 时值在 `u64` 上界附近**饱和**时，
    /// `to_smf_bytes` 必须给出明确的 `Err`，⛔ 不许 wrap、也不许 panic。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `track_events` 里的
    /// `start.saturating_add(note.duration_ticks)` 换成裸 `+`（注入 M4）后，
    /// 本 crate 的 **118** 条判据**全绿**（0 failed、退出码 0）⇒ 那个饱和加法
    /// 当时没有任何判据钉住。
    #[test]
    fn a_note_end_that_saturates_is_rejected_not_wrapped() {
        // 起点已经大于 VLQ 上限 ⇒ 第一颗事件本身就是 `DeltaOverflow`。
        // 关键在**时值那一步不许溢出**：`start + 100` 在 debug 构建会 panic。
        let start = u64::MAX - 10;
        let source = export(
            MidiFormat::SingleTrack,
            vec![track_from_notes("", 0, &[note(start, 60, 100, 64)])],
        );
        assert_eq!(
            source.to_smf_bytes(),
            Err(MidiError::DeltaOverflow {
                tick: start,
                delta: start
            })
        );
    }

    /// 判据 (类别④ 参数极值 / 类别⑦ 累加溢出): 正的 `micro_timing_ticks` 把起点推到
    /// `u64` 上界之外时，起点**饱和**成 `u64::MAX`，随后是明确的 `DeltaOverflow`。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `effective_start` 的正偏移分支换成裸 `+`
    /// （注入 M6）后，**118** 条判据全绿 ⇒ 只有负偏移的下溢有判据
    /// （`micro_timing_shifts_the_start_tick_without_underflow`），上溢那一半没有。
    #[test]
    fn a_positive_micro_offset_that_saturates_is_rejected_not_wrapped() {
        let start = u64::MAX - 5;
        let mut shifted = note(start, 60, 120, 64);
        shifted.micro_timing_ticks = Some(10);
        assert_eq!(
            effective_start(&shifted),
            u64::MAX,
            "起点必须饱和到 u64 的上界，不许绕回小 tick"
        );
        assert_eq!(
            export(
                MidiFormat::SingleTrack,
                vec![track_from_notes("", 0, &[shifted])]
            )
            .to_smf_bytes(),
            Err(MidiError::DeltaOverflow {
                tick: u64::MAX,
                delta: u64::MAX
            })
        );
    }

    /// 判据 (类别⑤ 幂等性 / `[ARCH-DET-001]`): **同一组** tempo map 记录，只把
    /// `tempos` 这个 `Vec` 的输入顺序换一下 ⇒ 导出字节**逐字节相同**。
    ///
    /// 补的是哪个缺口（本票注入实测）：删掉 `to_smf_bytes` 里那句
    /// `groups.sort_unstable()`（注入 M10 —— 换成一个恒等的比较器以保持可编译）后，
    /// **118** 条判据全绿 ⇒ "导出字节只由内容决定"当时只被
    /// `export_is_byte_deterministic`（同一份 `Vec` 导两次）覆盖，而那一条**看不见**
    /// 输入顺序。本条同时钉住配对：同一 tick 上"只有 tempo"与"只有拍号"的两条记录
    /// 不得被回读成**一条**合并记录（旧顺序下它们会相邻 ⇒ 拍号会挂到 tempo 上）。
    #[test]
    fn one_tempo_map_content_in_two_input_orders_is_byte_identical() {
        let tempo_only = MidiTempo {
            tick: 0,
            microseconds_per_quarter: Some(600_000),
            numerator: None,
            denominator_pow2: None,
        };
        let signature_only = MidiTempo {
            tick: 0,
            microseconds_per_quarter: None,
            numerator: Some(3),
            denominator_pow2: Some(2),
        };
        let build = |first: MidiTempo, second: MidiTempo| MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![first, second],
            tracks: vec![track_from_notes("T", 0, &[note(0, 60, 960, 64)])],
        };
        let forward = build(tempo_only, signature_only);
        let reverse = build(signature_only, tempo_only);
        let first = forward.to_smf_bytes().expect("编码");
        let second = reverse.to_smf_bytes().expect("编码");
        assert_eq!(
            first, second,
            "同一组 tempo 记录的两种输入顺序必须写出同一份字节"
        );
        for bytes in [&first, &second] {
            let parsed = parse_smf(bytes).expect("回读");
            assert_eq!(parsed.tempos.len(), 2, "两条记录必须各自回来, 不许被合并");
            assert!(
                parsed
                    .tempos
                    .iter()
                    .any(|tempo| tempo.microseconds_per_quarter == Some(600_000)
                        && tempo.numerator.is_none()),
                "只有 tempo 的记录必须原样回来: {:?}",
                parsed.tempos
            );
            assert!(
                parsed
                    .tempos
                    .iter()
                    .any(|tempo| tempo.microseconds_per_quarter.is_none()
                        && tempo.numerator == Some(3)),
                "只有拍号的记录必须原样回来: {:?}",
                parsed.tempos
            );
        }
    }

    /// 判据 (类别⑥ 通道一致性 / 格式号): `MidiFormat` 的**格式号**与反解互为逆。
    ///
    /// 这两个公开方法是 `MThd` 那个 `u16` 格式号的唯一权威映射。补的是哪个缺口
    /// （本票注入实测）：把 `number()` 的 0/1 两臂互换、把 `from_number()` 的
    /// 0/1 两臂互换，**4 次注入全部全绿** —— 本 crate 内部对这两个函数**零调用点**
    /// （`to_smf_bytes` 直接 `match` 枚举、`parse_smf` 直接构造枚举）
    /// ⇒ 这对公开函数此前既无判据也无覆盖。
    #[test]
    fn midi_format_numbers_round_trip_through_from_number() {
        assert_eq!(MidiFormat::SingleTrack.number(), 0, "SMF 格式 0");
        assert_eq!(MidiFormat::Parallel.number(), 1, "SMF 格式 1");
        for format in [MidiFormat::SingleTrack, MidiFormat::Parallel] {
            assert_eq!(
                MidiFormat::from_number(format.number()),
                Some(format),
                "{format:?} 的格式号必须能原样反解回来"
            );
        }
        assert_eq!(
            MidiFormat::from_number(2),
            None,
            "格式 2 (Sequential) 不在本枚举里 ⇒ 明确的 None"
        );
        assert_eq!(MidiFormat::from_number(u16::MAX), None);
    }

    /// 判据 (类别⑥ 通道一致性 / 字节全序): 同一 tick 上音符的**关闭**必须先于
    /// **开启**写出 —— 跨通道时也一样。
    ///
    /// `RawEvent::tie_break` 给 `NoteOff` 的 rank 是 `0`、`NoteOn` 是 `1`，
    /// 这条全序就是"先关后开"的实现。补的是哪个缺口（本票注入实测）：把
    /// `NoteOff` 的 rank 从 `0` 改成 `1`（注入 M21）后全部判据**保持绿** ——
    /// 既有的 `note_off_precedes_note_on_at_the_same_tick` 用的是**同通道同音高**
    /// 的两颗音符，那种情形下 rank 相同也仍按 `key < (key << 8)` 排对。
    ///
    /// ⚠️ 只有 `NoteOff` 的通道号**大于** `NoteOn` 的通道号时，rank 才是唯一的分辨者：
    /// 同 rank 时通道号先比较 ⇒ 关闭在**低**通道上时两种 rank 排出同一个顺序。
    /// 本判据因此把关闭放在通道 **1**、开启放在通道 **0**。
    #[test]
    fn a_note_off_precedes_a_note_on_of_another_channel_at_the_same_tick() {
        let source = export(
            MidiFormat::SingleTrack,
            vec![
                track_from_notes("High", 1, &[note(0, 60, 480, 100)]),
                track_from_notes("Low", 0, &[note(480, 62, 480, 100)]),
            ],
        );
        let bytes = source.to_smf_bytes().expect("导出");
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        let payload = &bytes[chunks[1].payload.clone()];

        let find = |needle: &[u8]| {
            payload
                .windows(needle.len())
                .position(|window| window == needle)
                .unwrap_or_else(|| panic!("负载里找不到 {needle:02X?}: {payload:02X?}"))
        };
        // 只按**状态字节 + 数据字节**定位（不带 delta：tick 480 上先出的那个事件的
        // delta 是 480 而不是 0 ⇒ 把 delta 写进 needle 会把判据钉在 delta 编码细节上）。
        let close = find(&[0x81, 0x3C, 0x00]); // NoteOff ch1 key 60
        let open = find(&[0x90, 0x3E, 0x64]); // NoteOn  ch0 key 62 vel 100
        assert!(
            close < open,
            "同一 tick 480 上必须先写 NoteOff (ch1) 再写 NoteOn (ch0); \
             实际 NoteOff 在 {close}, NoteOn 在 {open}: {payload:02X?}"
        );
    }

    /// 判据 (类别⑦ 块长度): 文件尾一个**负载长度 0** 的 chunk（它的头恰好占满
    /// 最后 8 字节）也必须被列出。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `track_chunks` 的循环条件从
    /// `cursor + 8 <= bytes.len()` 收紧成 `<`（注入 M32）后全部判据**保持绿**
    /// —— 已提交夹具里最后一条 `MTrk` 的负载都非空，"头落在文件尾"这一步没被走到。
    /// 本判据同时是 `TrackChunk::len()` / `is_empty()` 的**零值面**。
    #[test]
    fn a_zero_length_chunk_at_the_end_of_the_file_is_listed() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x03, 0xC0]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(bytes.len(), 22, "MThd 头 8 + 负载 6 + 空 MTrk 头 8");

        let chunks = track_chunks(&bytes).expect("两个 chunk 都必须被列出");
        assert_eq!(chunks.len(), 2, "尾部负载为 0 的 chunk 不许被静默丢掉");
        assert_eq!(&chunks[1].fourcc, b"MTrk");
        assert_eq!(chunks[1].len(), 0);
        assert!(chunks[1].is_empty());
        assert_eq!(chunks[1].payload, 22..22, "空负载的范围是空的");
    }

    /// 判据 (类别③ 静默丢弃 vs 明确 Err / 错误文案): `MidiError` 的 **14** 个变体
    /// 各有一个**字面** `Display` 读数。
    ///
    /// 补的是哪个缺口（本票注入实测）：把每一个变体的文案各改坏一次（注入
    /// A01..A14），**14 次全部全绿** ⇒ 本 crate 的 `tests/` 里对 `MidiError` 的
    /// `to_string()` / `Display` 引用次数此前是 **0**。
    ///
    /// ⚠️ 期望值一律是**字面字符串**（⛔ 不与 `DEFAULT_PPQ` / `VLQ_MAX` 之类的常量自比：
    /// 那样在常量被改时恒真）。
    #[test]
    fn midi_error_display_text_is_pinned_for_every_variant() {
        let zero = entity_id("00000000000000000000000000").expect("ULID");
        let cases: Vec<(MidiError, &str)> = vec![
            (MidiError::InvalidPpq(0), "PPQ 非法: 0"),
            (MidiError::ChannelOutOfRange(16), "通道号越界: 16"),
            (MidiError::PitchOutOfRange(128), "音高越界: 128"),
            (MidiError::VelocityOutOfRange(200), "力度越界: 200"),
            (
                MidiError::ZeroDuration { note: zero },
                "音符 00000000000000000000000000 的时值为 0",
            ),
            (
                MidiError::DeltaOverflow { tick: 7, delta: 9 },
                "tick 7 处的 delta 9 超过 VLQ 的 28 位上限",
            ),
            (MidiError::NoTracks, "没有可导出的轨道"),
            (MidiError::Encode("boom".to_owned()), "SMF 编码失败: boom"),
            (MidiError::Decode("bad".to_owned()), "SMF 解析失败: bad"),
            (MidiError::UnsupportedTimecode, "不支持 SMPTE 时间码分度"),
            (MidiError::UnsupportedFormat(2), "不支持的 SMF 格式号: 2"),
            (
                MidiError::UnclosedNote {
                    start_tick: 480,
                    key: 60,
                },
                "tick 480 的音符 (key 60) 没有 NoteOff",
            ),
            (
                MidiError::UnmatchedNoteOff { tick: 960, key: 61 },
                "tick 960 的 NoteOff (key 61) 没有对应 NoteOn",
            ),
            (
                MidiError::HalfTimeSignature {
                    tick: 0,
                    numerator: Some(4),
                    denominator_pow2: None,
                },
                "tick 0 的拍号只给了一半: numerator = Some(4), denominator_pow2 = None",
            ),
        ];
        assert_eq!(cases.len(), 14, "MidiError 的变体数");
        assert_every_midi_error_arm_is_covered(&cases);
        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected, "{error:?} 的 Display 文案");
        }

        // ⚠️ **登记（未被注入验证）**：`MidiError` 的 `Error::source()` 恒为 `None`。
        // 那是 std 默认实现的默认行为，而 `impl std::error::Error for MidiError {}`
        // 里没有可做字面替换的臂 ⇒ 本票**没有**能打它的注入。这一句是
        // **依赖/默认行为钉子**，⛔ 不计入"已注入验证"。
        assert!(
            std::error::Error::source(&MidiError::NoTracks).is_none(),
            "MidiError 没有内层错误 ⇒ source() 必须是 std 的默认 None"
        );
    }

    /// 判据 (类别① 越界输入 / 块布局): `track_chunks` **如实**返回任何 fourcc
    /// （⛔ 不按 `MThd`/`MTrk` 白名单过滤），越界时错误文案**点名**那个 fourcc。
    ///
    /// 补的是哪个缺口（本票注入实测）：① 只保留 `MThd`/`MTrk`、丢掉其它 fourcc
    /// （注入 C01）与 ② 文案里少一个逗号（注入 C03）**都全绿** —— 已提交的判据只在
    /// `fourcc` 上断言过 `MThd` 与 `MTrk` 两个值，越界文案只被 `contains("声明 999 字节")`
    /// 这种**片段**断言碰过。
    #[test]
    fn track_chunks_keeps_unknown_fourcc_bytes_verbatim() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&6u32.to_be_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0x00, 0x01, 0x03, 0xC0]);
        bytes.extend_from_slice(b"XxXx");
        bytes.extend_from_slice(&2u32.to_be_bytes());
        bytes.extend_from_slice(&[0xAB, 0xCD]);
        bytes.extend_from_slice(b"MTrk");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        assert_eq!(bytes.len(), 32, "14 + (8 + 2) + 8");

        let chunks = track_chunks(&bytes).expect("三个 chunk 都必须被列出");
        assert_eq!(chunks.len(), 3, "未知 fourcc 的 chunk 不许被过滤掉");
        assert_eq!(&chunks[1].fourcc, b"XxXx");
        assert_eq!(chunks[1].len(), 2);
        assert_eq!(&bytes[chunks[1].payload.clone()], &[0xAB, 0xCD]);

        // 越界：`XxXx` 声明 2 字节，文件在它之后只剩 1 字节。
        let mut lying = bytes.clone();
        lying.truncate(23);
        match track_chunks(&lying) {
            Err(MidiError::Decode(message)) => assert_eq!(
                message, "chunk XxXx 声明 2 字节, 但文件只剩 1 字节",
                "错误文案必须点名 fourcc 与两个长度（字面读数）"
            ),
            other => panic!("期望 Decode, 得到 {other:?}"),
        }
    }

    /// 判据 (类别: 公开 **`Debug` 形状** —— `Display` 之后的第二族诊断面):
    /// 四个公开结果类型的派生 `Debug` 输出**逐字面**钉住。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `TrackChunk` / `MidiTempo` / `ParsedNote` /
    /// `MusicXmlNote` 的 `#[derive(Debug)]` 各顶成一个写死的手写 `impl Debug`
    /// （注入 DBG01..DBG04）后全部判据**保持绿** —— `{x:?}` 被大量 `assert_eq!` 的
    /// **失败消息**用到，但**没有任何判据断言过它的形状**。
    ///
    /// ⚠️ 派生 `Debug` 的形状由**类型定义**决定 ⇒ 单点字面替换打不到它；要打就必须像
    /// 本批那样**插入一个手写 `impl Debug` 顶掉 derive**。
    #[test]
    fn public_result_debug_shapes_are_pinned() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"MThd");
        bytes.extend_from_slice(&0u32.to_be_bytes());
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(
            format!("{:?}", chunks[0]),
            "TrackChunk { fourcc: [77, 84, 104, 100], payload: 8..8 }"
        );
        assert_eq!(
            format!(
                "{:?}",
                MidiTempo {
                    tick: 0,
                    microseconds_per_quarter: Some(500000),
                    numerator: Some(4),
                    denominator_pow2: Some(2),
                }
            ),
            "MidiTempo { tick: 0, microseconds_per_quarter: Some(500000), \
             numerator: Some(4), denominator_pow2: Some(2) }"
        );
        assert_eq!(
            format!(
                "{:?}",
                ParsedNote {
                    channel: 0,
                    key: 60,
                    velocity: 100,
                    start_tick: 0,
                    end_tick: 480,
                }
            ),
            "ParsedNote { channel: 0, key: 60, velocity: 100, start_tick: 0, end_tick: 480 }"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::musicxml::MusicXmlNote {
                    key: 60,
                    velocity: 80,
                    start_tick: 0,
                    duration_ticks: 480,
                    voice: 1,
                    staff: 1,
                }
            ),
            "MusicXmlNote { key: 60, velocity: 80, start_tick: 0, \
             duration_ticks: 480, voice: 1, staff: 1 }"
        );
    }

    /// 判据 (类别: **核对次序** —— 本批新开的一族契约): `validate_note` 的四条检查有
    /// **固定优先级**，通道在最前。
    ///
    /// 补的是哪个缺口（本票注入实测）：把通道检查与音高检查**对调**（注入 ORD01）后
    /// 全部判据**保持绿** —— 既有判据每次只弄坏**一个**字段 ⇒ "两个都坏时报哪一条"
    /// 此前没有判据（与 `the_length_check_runs_before_the_crc_check` 同一族）。
    #[test]
    fn the_note_validation_order_is_pinned() {
        let source = MidiExport {
            format: MidiFormat::SingleTrack,
            ppq: DEFAULT_PPQ,
            tempos: Vec::new(),
            tracks: vec![MidiExportTrack {
                name: String::new(),
                channel: 16,
                notes: vec![note(0, 200, 480, 100)],
            }],
        };
        assert_eq!(
            source.to_smf_bytes(),
            Err(MidiError::ChannelOutOfRange(16)),
            "通道与音高同时越界 ⇒ 必须先报通道"
        );
    }

    /// 判据 (类别: 核对次序): `to_smf_bytes` 先查**时间分度**、再查**轨道表**。
    ///
    /// 补的是哪个缺口（本票注入实测）：把两条对调（注入 ORD02）后全部判据**保持绿**
    /// —— 既有判据分别构造 `ppq = 0` 与空轨道，从不同时给两个坏条件。
    #[test]
    fn a_zero_ppq_is_reported_before_an_empty_track_list() {
        let source = MidiExport {
            format: MidiFormat::SingleTrack,
            ppq: 0,
            tempos: Vec::new(),
            tracks: Vec::new(),
        };
        assert_eq!(
            source.to_smf_bytes(),
            Err(MidiError::InvalidPpq(0)),
            "分度 0 与空轨道同时成立 ⇒ 必须先报 InvalidPpq"
        );
    }

    /// **编译期穷举探针**：给 `MidiError` 的每个变体一个唯一编号 ⇒ **新增一个变体**
    /// 就让这个 `match` 变成非穷举、**编译失败**。
    ///
    /// ⚠️ 为什么需要它：黄金表里的 `assert_eq!(cases.len(), 14, …)` 只自校验**表**的
    /// 长度，⛔ 读不到枚举本身 —— "加了变体、也加了产线 `Display` 臂、却忘了往表里加
    /// 一行"这种情形它抓不到（表长仍是 14）。下面的
    /// `assert_every_midi_error_arm_is_covered` 把表与穷举探针绑在一起。
    /// ⛔ **不许给这个 `match` 加 `_ =>` 通配臂**（R51）：加了以后新增变体也能编译过，
    /// 探针立刻**静默失效**，而**所有判据仍然全绿**。
    fn midi_error_arm(error: &MidiError) -> u8 {
        match error {
            MidiError::InvalidPpq(_) => 0,
            MidiError::ChannelOutOfRange(_) => 1,
            MidiError::PitchOutOfRange(_) => 2,
            MidiError::VelocityOutOfRange(_) => 3,
            MidiError::ZeroDuration { .. } => 4,
            MidiError::DeltaOverflow { .. } => 5,
            MidiError::NoTracks => 6,
            MidiError::Encode(_) => 7,
            MidiError::Decode(_) => 8,
            MidiError::UnsupportedTimecode => 9,
            MidiError::UnsupportedFormat(_) => 10,
            MidiError::UnclosedNote { .. } => 11,
            MidiError::UnmatchedNoteOff { .. } => 12,
            MidiError::HalfTimeSignature { .. } => 13,
        }
    }

    /// 黄金表必须**逐臂恰好一次**（缺一臂 ⇒ 编号集合不完整 ⇒ 红）。
    fn assert_every_midi_error_arm_is_covered(cases: &[(MidiError, &str)]) {
        let mut arms: Vec<u8> = cases
            .iter()
            .map(|(error, _)| midi_error_arm(error))
            .collect();
        arms.sort_unstable();
        assert_eq!(
            arms,
            (0..14).collect::<Vec<u8>>(),
            "黄金表必须逐臂恰好一次（缺一臂或不重复都红）"
        );
    }

    /// 判据 (类别: 公开 **`Debug` 形状**，续): 其余 **8** 个公开类型的派生 `Debug`
    /// 输出逐字面钉住（`MidiExportTrack` / `MidiExport` / `MidiFormat` / `MidiError` /
    /// `ParsedMidi` / `MusicXmlPart` / `MusicXmlScore` / `MxlLimits`）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把这 8 个类型的 `#[derive(Debug)]` 各顶成一个
    /// 写死的手写 `impl Debug`（注入 DBG05..DBG12）后全部判据**保持绿**。
    /// ⚠️ 派生 `Debug` 的形状由类型定义决定 ⇒ 只能靠**插入手写 `impl` 顶掉 derive**
    /// 才造得出牙（与第七批同一机制）。
    /// ⛔ `CentralEntry` 不在此列：它是**私有**结构体，它的 `Debug` 不是公开面。
    #[test]
    fn more_public_debug_shapes_are_pinned() {
        assert_eq!(format!("{:?}", MidiFormat::SingleTrack), "SingleTrack");
        assert_eq!(format!("{:?}", MidiFormat::Parallel), "Parallel");
        assert_eq!(format!("{:?}", MidiError::NoTracks), "NoTracks");
        assert_eq!(format!("{:?}", MidiError::InvalidPpq(0)), "InvalidPpq(0)");
        assert_eq!(
            format!(
                "{:?}",
                MidiExportTrack {
                    name: "A".to_owned(),
                    channel: 0,
                    notes: Vec::new(),
                }
            ),
            "MidiExportTrack { name: \"A\", channel: 0, notes: [] }"
        );
        assert_eq!(
            format!(
                "{:?}",
                MidiExport {
                    format: MidiFormat::SingleTrack,
                    ppq: 960,
                    tempos: Vec::new(),
                    tracks: Vec::new(),
                }
            ),
            "MidiExport { format: SingleTrack, ppq: 960, tempos: [], tracks: [] }"
        );
        assert_eq!(
            format!(
                "{:?}",
                ParsedMidi {
                    format: MidiFormat::Parallel,
                    ppq: 480,
                    tempos: Vec::new(),
                    notes: Vec::new(),
                }
            ),
            "ParsedMidi { format: Parallel, ppq: 480, tempos: [], notes: [] }"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::musicxml::MusicXmlPart {
                    id: "P".to_owned(),
                    name: "N".to_owned(),
                    notes: Vec::new(),
                }
            ),
            "MusicXmlPart { id: \"P\", name: \"N\", notes: [] }"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::musicxml::MusicXmlScore {
                    divisions: 1,
                    ppq: 960,
                    tempos: Vec::new(),
                    parts: Vec::new(),
                    ignored_elements: std::collections::BTreeMap::new(),
                    unsupported_elements: std::collections::BTreeMap::new(),
                }
            ),
            "MusicXmlScore { divisions: 1, ppq: 960, tempos: [], parts: [], \
             ignored_elements: {}, unsupported_elements: {} }"
        );
        assert_eq!(
            format!(
                "{:?}",
                crate::mxl::MxlLimits {
                    max_entry_bytes: 1,
                    max_entries: 2,
                    max_name_bytes: 3,
                }
            ),
            "MxlLimits { max_entry_bytes: 1, max_entries: 2, max_name_bytes: 3 }"
        );
    }

    /// 判据 (类别: **`PartialEq` 语义 / 浮点自反性**): `MidiExportTrack` 与 `MidiExport`
    /// 因 `MidiNote::probability: Option<f32>` **只有 `PartialEq`**（没有 `Eq`）
    /// ⇒ 含 `NaN` 概率的音符让**整个导出输入不等于一份字段完全相同的拷贝**。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `MidiExportTrack` / `MidiExport` 的
    /// `#[derive(PartialEq)]` 各顶成一个只看 `name` / `ppq` 的手写 `impl PartialEq`
    /// （注入 PEQ01/PEQ02）后全部判据**保持绿**。
    ///
    /// ⚠️ 这是**登记在案的语言级现状**（`f32::NAN != f32::NAN`），⛔ 不是缺陷；
    /// 钉住它是为了让"某天有人给这两个类型手写 `PartialEq`／加上 `Eq`"**可观测**。
    /// ⛔ 本判据**不用** `x == x` 写法（会触发 `clippy::eq_op`），而是构造两份字段
    /// 完全相同、但概率是 `NaN` 的值。
    #[test]
    fn an_export_track_with_a_nan_probability_is_not_equal_to_itself() {
        let note_id = entity_id("00000000000000000000000000").expect("ULID");
        let with_nan = || MidiExportTrack {
            name: "A".to_owned(),
            channel: 0,
            notes: vec![MidiNote {
                probability: Some(f32::NAN),
                ..MidiNote::new(note_id, 0, 60, 480)
            }],
        };
        assert!(
            with_nan() != with_nan(),
            "两份字段完全相同、概率是 NaN 的轨道**不相等** ⇒ NaN 的自反性不成立"
        );

        let export = || MidiExport {
            format: MidiFormat::SingleTrack,
            ppq: DEFAULT_PPQ,
            tempos: Vec::new(),
            tracks: vec![with_nan()],
        };
        assert!(
            export() != export(),
            "外层 `MidiExport` 的相等语义同样被 NaN 污染（它逐字段委托给轨道）"
        );

        // 对照臂：把 NaN 换成有限值 ⇒ 自反性回来（证明上面红的是 NaN、不是别的东西）。
        let finite = || MidiExportTrack {
            name: "A".to_owned(),
            channel: 0,
            notes: vec![MidiNote {
                probability: Some(0.5),
                ..MidiNote::new(note_id, 0, 60, 480)
            }],
        };
        assert!(finite() == finite(), "有限概率 ⇒ 两份相同的轨道必须相等");
    }

    /// 判据 (类别: `PartialEq` 的**浮点面**): `-0.0` 与 `+0.0` **相等**、次正规数与自身
    /// **相等** —— `PartialEq` 用的是 IEEE 的 `==`（⛔ 不是按位比较）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `MidiExportTrack` 的 `PartialEq` 改成比较概率的
    /// **位模式**（`f32::to_bits`，注入 `b10:FLT03`）后全部判据里只有本批之前的 NaN 判据变红；
    /// 而**按位比较**与 **NaN 判据**是两件不同的事：一个"字段全等才算相等"的实现会让
    /// `-0.0` 与 `+0.0` 不等，而 NaN 那一侧的反自反性仍然成立。
    #[test]
    fn float_sign_and_subnormals_do_not_break_track_equality() {
        let note_id = entity_id("00000000000000000000000000").expect("ULID");
        let track = |probability: f32| MidiExportTrack {
            name: "A".to_owned(),
            channel: 0,
            notes: vec![MidiNote {
                probability: Some(probability),
                ..MidiNote::new(note_id, 0, 60, 480)
            }],
        };

        // IEEE：`-0.0 == 0.0` ⇒ 两个**符号位不同**的轨道必须相等。
        assert!(
            track(-0.0) == track(0.0),
            "-0.0 与 +0.0 必须相等（⛔ 不是按位比较）"
        );

        // 次正规数（最小正正规数的一半）与自身相等。
        let subnormal = f32::MIN_POSITIVE / 2.0;
        assert!(
            subnormal > 0.0 && subnormal < f32::MIN_POSITIVE,
            "前提：这个值确实是次正规数"
        );
        assert!(
            track(subnormal) == track(subnormal),
            "次正规数与自身必须相等"
        );

        // 反向臂：真正不同的值必须不等。
        assert!(track(0.1) != track(0.2), "不同概率必须不等");
        // 反向臂：`None` 与 `Some(0.0)` 必须不等。
        let none = MidiExportTrack {
            name: "A".to_owned(),
            channel: 0,
            notes: vec![MidiNote::new(note_id, 0, 60, 480)],
        };
        assert!(none != track(0.0), "None 与 Some(0.0) 必须不等");
    }

    /// 判据 (类别: `Clone` 的**深拷贝**语义): `MidiExportTrack::clone` 必须把
    /// `Vec<MidiNote>` 里的载荷（以及 `MidiNote` 自己的 `String` / `Vec` 载荷）
    /// **真的复制**一份 —— 改克隆不影响原件。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `Clone` 顶成一个丢掉 `notes` 的手写实现
    /// （注入 `b10:CLN01`）后全部判据**保持绿** ⇒ 本 crate 此前没有任何判据查过
    /// `clone()` 的**深拷贝**语义（判据都用 `clone()`，但从没验证它复制了载荷）。
    #[test]
    fn clone_of_an_export_track_copies_every_payload() {
        let note_id = entity_id("00000000000000000000000000").expect("ULID");
        let mut note = MidiNote::new(note_id, 0, 60, 480);
        note.probability = Some(0.5);
        note.syllable = Some("la".to_owned());
        note.phonemes = vec!["l".to_owned(), "a".to_owned()];
        note.pitch_bend_curve = vec![(0, 0), (240, 100)];
        let original = MidiExportTrack {
            name: "A".to_owned(),
            channel: 3,
            notes: vec![note],
        };

        let mut clone = original.clone();
        assert!(clone == original, "克隆必须与原件相等");
        assert_eq!(clone.notes.len(), 1, "音符必须被复制");
        assert_eq!(clone.notes[0].syllable.as_deref(), Some("la"));
        assert_eq!(clone.notes[0].phonemes.len(), 2);
        assert_eq!(clone.notes[0].pitch_bend_curve.len(), 2);

        // 改克隆 ⇒ 原件必须不受影响（深拷贝的第一条直接证据）。
        clone.notes[0].syllable = Some("changed".to_owned());
        clone.notes[0].phonemes.clear();
        clone.notes[0].pitch_bend_curve.clear();
        clone.notes[0].probability = None;
        clone.name.push('!');
        assert_eq!(original.notes.len(), 1, "原件仍有 1 颗音符");
        assert_eq!(original.notes[0].syllable.as_deref(), Some("la"));
        assert_eq!(original.notes[0].phonemes.len(), 2);
        assert_eq!(original.notes[0].pitch_bend_curve.len(), 2);
        assert_eq!(original.notes[0].probability, Some(0.5));
        assert_eq!(original.name, "A");
    }

    /// 判据 (类别: `PartialEq` 的**顺序敏感**面): `MidiExport` 的相等用的是 `Vec` 的
    /// 逐元素比较 ⇒ **同一批轨道/速度记录换个顺序就不相等**（⛔ 不是多重集语义）。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `MidiExport` 的 `PartialEq` 顶成一个
    /// "按 `name` 排序后再比较"的手写实现（注入 `b11:FLT04`）后全部判据**保持绿**
    /// ⇒ 这个顺序敏感面此前没有判据。
    #[test]
    fn export_equality_is_sensitive_to_track_order() {
        let lane = |name: &str, channel: u8| MidiExportTrack {
            name: name.to_owned(),
            channel,
            notes: Vec::new(),
        };
        let forward = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: Vec::new(),
            tracks: vec![lane("A", 0), lane("B", 1)],
        };
        let reversed = MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: Vec::new(),
            tracks: vec![lane("B", 1), lane("A", 0)],
        };
        assert!(forward == forward.clone(), "对照臂：同一顺序必须相等");
        assert!(
            forward != reversed,
            "换个顺序必须不相等（`Vec` 是顺序敏感的）"
        );

        // `tempos` 同样顺序敏感。
        let with_tempos = |first: u64, second: u64| MidiExport {
            format: MidiFormat::Parallel,
            ppq: DEFAULT_PPQ,
            tempos: vec![
                MidiTempo {
                    tick: first,
                    microseconds_per_quarter: Some(500_000),
                    numerator: None,
                    denominator_pow2: None,
                },
                MidiTempo {
                    tick: second,
                    microseconds_per_quarter: Some(600_000),
                    numerator: None,
                    denominator_pow2: None,
                },
            ],
            tracks: Vec::new(),
        };
        assert!(
            with_tempos(0, 100) != with_tempos(100, 0),
            "`tempos` 也是顺序敏感的"
        );
    }

    /// 判据 (R75: **常量的字面钉子**): 本 crate 的公开常量一律用**字面值**钉住。
    ///
    /// 补的是哪个缺口（本票注入实测）：把 `musicxml::DEFAULT_VELOCITY` 从 `80` 改成 `81`
    /// （注入 `b11:CONST01`）后全部判据**保持绿** ⇒ 该常量此前只有"**与自己比**"的断言
    /// （`musicxml_contract.rs` 里的 `.all(|n| n.velocity == DEFAULT_VELOCITY)`）——
    /// 那是**常量自比**：常量改了、期望值跟着改、恒真。
    #[test]
    fn public_constants_are_pinned_by_literals() {
        assert_eq!(
            DEFAULT_PPQ, 960,
            "工程与 SMF 的时间分度基准 [MODEL-AST-001]"
        );
        assert_eq!(crate::musicxml::DEFAULT_VELOCITY, 80, "MusicXML 默认力度");
        assert_eq!(crate::musicxml::MAX_DEPTH, 256, "元素嵌套深度上限");
        assert_eq!(crate::vlq::VLQ_MAX, 0x0FFF_FFFF, "VLQ 的 28 位上界");
        assert_eq!(crate::vlq::VLQ_MAX_BYTES, 4, "VLQ 的最大字节数");
        assert_eq!(crate::export::MIDI_CHANNEL_COUNT, 16, "SMF 的通道数");
    }
}
