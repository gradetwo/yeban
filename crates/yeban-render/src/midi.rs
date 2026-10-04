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

/// 一条 tempo 事件。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MidiTempo {
    /// 绝对 tick。
    pub tick: u64,
    /// 每四分音符的微秒数 (`mpqn`)。
    pub microseconds_per_quarter: u32,
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
            microseconds_per_quarter: mpqn,
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
    /// Tempo map（按 tick 升序; 会被自动排序）。
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
        }
    }
}

impl std::error::Error for MidiError {}

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
        microseconds_per_quarter: u32,
    },
    TimeSignature {
        tick: u64,
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

    /// 同一 tick 内的全序键: `(rank, channel, key, velocity/值)`。
    ///
    /// 必须给出**全序**, 否则同一 tick 的多个事件会按输入顺序落到文件里,
    /// 让"同一工程两次导出字节不同"。
    fn tie_break(self) -> (u8, u8, u8, u32) {
        match self {
            Self::NoteOff { channel, key, .. } => (0, channel, key, 0),
            Self::NoteOn {
                channel,
                key,
                velocity,
                ..
            } => (1, channel, key, u32::from(velocity)),
            Self::Tempo {
                microseconds_per_quarter,
                ..
            } => (2, 0, 0, microseconds_per_quarter),
            Self::TimeSignature {
                numerator,
                denominator_pow2,
                ..
            } => (3, 0, numerator, u32::from(denominator_pow2)),
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
        if self.ppq == 0 {
            return Err(MidiError::InvalidPpq(self.ppq));
        }
        if self.tracks.is_empty() {
            return Err(MidiError::NoTracks);
        }

        // tempo map 独立于轨道, 先规范化。
        let mut tempo_events: Vec<RawEvent> = Vec::new();
        for tempo in &self.tempos {
            tempo_events.push(RawEvent::Tempo {
                tick: tempo.tick,
                microseconds_per_quarter: tempo.microseconds_per_quarter.min(0x00FF_FFFF),
            });
            if let (Some(numerator), Some(denominator_pow2)) =
                (tempo.numerator, tempo.denominator_pow2)
            {
                tempo_events.push(RawEvent::TimeSignature {
                    tick: tempo.tick,
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
                    microseconds_per_quarter: mpqn.as_int(),
                    numerator: None,
                    denominator_pow2: None,
                }),
                TrackEventKind::Meta(MetaMessage::TimeSignature(
                    numerator,
                    denominator_pow2,
                    _clocks,
                    _notes,
                )) => {
                    if let Some(last) = tempos.iter_mut().rev().find(|tempo| tempo.tick == tick) {
                        last.numerator = Some(numerator);
                        last.denominator_pow2 = Some(denominator_pow2);
                    } else {
                        tempos.push(MidiTempo {
                            tick,
                            microseconds_per_quarter: 500_000,
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
        assert_eq!(chunks[0].payload, 0..6);
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
            parsed.tempos[0].microseconds_per_quarter, 500_000,
            "120 BPM"
        );
        assert_eq!(parsed.tempos[0].numerator, Some(4));
        assert_eq!(parsed.tempos[0].denominator_pow2, Some(2));
        assert_eq!(parsed.tempos[1].tick, 960);
        assert_eq!(parsed.tempos[1].microseconds_per_quarter, 666_667, "90 BPM");
        // conductor 轨道存在 -> 格式 1 有两条 MTrk
        let chunks = track_chunks(&bytes).expect("chunk 布局");
        assert_eq!(chunks.len(), 3, "MThd + conductor + 一条音符轨");
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
        assert_eq!(MidiTempo::from_bpm(120.0).microseconds_per_quarter, 500_000);
        assert_eq!(MidiTempo::from_bpm(90.0).microseconds_per_quarter, 666_667);
        assert_eq!(
            MidiTempo::from_bpm(60.0).microseconds_per_quarter,
            1_000_000
        );
        assert_eq!(
            MidiTempo::from_bpm(0.0).microseconds_per_quarter,
            500_000,
            "非法 BPM 回退到 120"
        );
        assert!(MidiTempo::from_bpm(1.0e12).microseconds_per_quarter <= 0x00FF_FFFF);
    }
}
