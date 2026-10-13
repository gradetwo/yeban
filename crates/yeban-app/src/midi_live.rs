//! ⭐ **连续性**：把**已经解析好的** MIDI 事件在**到达的那一刻**泵进引擎的音符路径。
//!
//! ## 为什么要有这个模块（⭐ 缺的是"连续性"，⛔ 不是"MIDI 输入"）
//!
//! "采到的消息 ⇒ 音乐"两条路已经落地：音符进工程（`Op::AddNote`，见
//! [`crate::midi_notes::write_notes`]）与声音进引擎（`EngineEvent` 环 ＋ 离线渲染，见
//! [`crate::midi_notes::render_notes_to_wav`]）。⚠ 但那两条都是**批次**的 ——
//! [`crate::midi_in::capture_messages`] 采集 `seconds` 秒、**到点才返回**，而它建的会话
//! （`yeban_midi_io::MidiInputSession`）在采集窗口结束时就**被丢弃**
//! （`crates/yeban-app/src/midi_in.rs:444-452`）⇒ 一次按键先被**缓冲**到窗口结束才被处理。
//!
//! ⭐ **本模块补的正是那一段**：一个**控制侧**的泵 ——
//! [`MidiEventReceiver`] ⇒（逐条映射）⇒ 既有的 UI→引擎 SPSC
//! （[`EngineHost::send_engine_events`]）⇒ 合成器。
//! ⛔ 它**不**新建第二条通路、⛔ 不新造 `Op`、⛔ 不碰 `schemas/**`：声音本来就是经
//! **既有的**事件环到合成器的，而那个环**从不接触模型**（"音符要不要落进工程"是产品的
//! 另一件事，仍由 [`crate::midi_notes::write_notes`] 负责）。
//!
//! ## 映射表（⭐ 逐条写死；与 [`crate::midi_notes::pair_notes`] 的判定**同一套**）
//!
//! | 状态字节 | 判定 | 产物 |
//! | :--- | :--- | :--- |
//! | `status & 0xF0 == 0x90` 且**力度 > 0**（`data2 & 0x7F`） | NoteOn | [`EngineEvent::NoteOn`]（`pitch = data1 & 0x7F`） |
//! | `status & 0xF0 == 0x90` 且**力度 == 0** | **NoteOff**（⭐ running status 的常见写法 ⇒ 这是**经典 bug 点**） | [`EngineEvent::NoteOff`] |
//! | `status & 0xF0 == 0x80` | NoteOff | [`EngineEvent::NoteOff`] |
//! | 其余（`0xA0`/`0xB0`/`0xC0`/`0xD0`/`0xE0`/系统消息） | **不是音符** | ⭐ **计数**（[`LivePumpReport::ignored`]），⛔ **不静默丢** |
//!
//! ⭐ **顺序**：弹出顺序 ＝ [`MidiEventReceiver`] 的 FIFO 顺序（`rtrb` 整事件原子，⛔ 看不到
//! 半条）⇒ 一批里的 `EngineEvent` **按到达顺序**排列 ⇒ 同一块内 `NoteOff` 不会越过它前面的
//! `NoteOn`（引擎侧 `drain_with` 也是 FIFO，见 `crates/yeban-engine/src/ring.rs:246`）。
//!
//! ## 实时安全（红线 7）：⭐ 本模块**整个在控制侧**，音频侧一位不动
//!
//! * ⭐ **本泵不分配、不加锁、不做 I/O**：两个临时缓冲都是**定长栈数组**
//!   （`[MidiEvent; PUMP_CHUNK]` / `[EngineEvent; PUMP_CHUNK]`）⇒ 搬运一轮零堆操作；
//! * ⭐ **发布走既有的批量 API**：[`EngineHost::send_engine_events`] ⇒ `EventSender::publish`
//!   ⇒ **恰好一次** `push_partial_slice`（`crates/yeban-engine/src/ring.rs:193`）；
//!   ⭐ 一条消息**一次**发布是**结构性错误** ⇒ 读数 [`LivePumpReport::publishes`] 把它钉住
//!   （`publishes` = 批量发布**调用次数**，⛔ 不是事件条数）；
//! * ⭐ **音频侧是 pop-only**：它每个量子**恰好一次**批量出队
//!   （`crates/yeban-engine/src/rt.rs:1960` 的 `event_bulk_pops`）—— 本模块**没有**给音频
//!   线程加任何东西（判据读 `EngineStats::event_bulk_pops` 的增量 ＝ 量子数）。
//!
//! ## 时间口径（⚠ 引擎事件的**既有**边界，⛔ 本模块不发明时间）
//!
//! [`EngineEvent`] **不带时间戳** ⇒ 泵进去的事件在**下一个块边界**生效
//! （`DEFAULT_BLOCK_FRAMES` = 128 帧 ≈ 2.7 ms @48 kHz）。⭐ "实时"因此是**一个量子**的
//! 粒度 —— 与设备回调、与 [`crate::midi_notes::render_engine_offline`] **同一个**粒度。
//! [`MidiEvent::timestamp_us`] 在泵里**不被读**：它在配对路（[`crate::midi_notes`]）里换成
//! tick，而音符路（环）从来只认"哪一块"。
//!
//! ## ⚠ 登记的断点（⛔ 不假装整条通）
//!
//! ⭐ 本模块提供的是**能力**（"把环里的事件泵进引擎"）；把它**周期性调用**的那个会话循环
//! （真端口 ⇒ 边采边泵）**只能在有 MIDI 硬件的机器上验** —— `midir` 的真回调线程在本机与
//! CI 都跑不到（与 `crates/yeban-midi-io/src/lib.rs` 记的断点同一条）。⭐ 本机可判的那层是
//! **"已解析的事件 ⇒ 引擎的音符路径"**，判据用**模拟回调体**
//! `MidiEventSender::handle_message`（＝真回调的**内核**，见
//! `crates/yeban-midi-io/src/input.rs:170`）喂真接收侧 ⇒ 泵 ⇒ 真引擎。

use yeban_engine::ring::EngineEvent;
use yeban_model::EntityId;

use crate::engine_host::EngineHost;

// ⭐ 这几个类型**不是**本模块新造的：它们就是 `yeban-midi-io` 的**既有**事件环
// （`crates/yeban-midi-io/src/input.rs`，`ADR-0006` D3）⇒ re-export 只是让调用方
// （含判据）从**一处**取它们，⛔ 不是第二条通路、⛔ 不是第二个环。
pub use yeban_midi_io::{MidiEvent, MidiEventReceiver, MidiEventSender, event_channel};

/// ⭐ 一次泵最多搬运的条数（环 ⇒ 环）。
///
/// ⭐ 它是**有界**的，因为泵是**连续**调的：一个灌进来的事件洪峰不该让一次调用无限搬运
/// （⛔ 不阻塞控制侧、⛔ 不让栈缓冲无界）。缓冲是栈上的定长数组 ⇒ 零分配。
pub const PUMP_CHUNK: usize = 128;

/// 临时缓冲的填充值（⚠ 只有在"环里没有事件"时留在缓冲里，⛔ 永不被当作一条事件读）。
const EMPTY_MIDI_EVENT: MidiEvent = MidiEvent {
    timestamp_us: 0,
    status: 0,
    data1: 0,
    data2: 0,
};

/// ⭐ 一条 MIDI 事件在**音符路径**上的身份（[`crate::midi_notes::pair_notes`] 的判定，
/// 这里是逐条版：⛔ 不配对、⛔ 不缓冲、⛔ 不算 tick）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteAction {
    /// 音符开（`0x9n` 且力度 > 0）。
    On {
        /// MIDI 音高（`data1 & 0x7F`）。
        pitch: u8,
        /// MIDI 力度（`data2 & 0x7F`，⚠ 恒 > 0 ⇒ 是一个**真的**按下的音符）。
        velocity: u8,
    },
    /// 音符关：`0x8n`，**以及** `0x9n` 且力度 == 0（⭐ running status 的常见写法）。
    Off {
        /// MIDI 音高（`data1 & 0x7F`）。
        pitch: u8,
    },
    /// ⭐ **不是**音符（CC / 弯音 / 触后 / 实时 / 系统）—— 调用方**计数**，⛔ 不静默丢。
    NotANote,
}

/// ⭐ **一条已解析的 MIDI 事件 ⇒ 它在音符路径上的身份**（纯函数：零 I/O、零状态）。
///
/// ⭐ 数据字节先按 `& 0x7F` 取真值，**判定与载荷用同一个值** ⇒ "力度 0" 这条规则只有
/// **一处**（`0x90` 分支上的 `velocity > 0`），⛔ 不会出现"判定说 NoteOn、载荷却是 0"。
///
/// ⚠ `midir` 保证数据字节 ≤ 127（`MidiEvent::from_message` 只做长度校验）；这里的掩码是
/// 为了**不**让一个越界字节变成幻觉音符（`data2 = 0x80` ⇒ 掩码后 0 ⇒ NoteOff）。
#[must_use]
pub fn note_action(event: MidiEvent) -> NoteAction {
    let pitch = event.data1 & 0x7F;
    let velocity = event.data2 & 0x7F;
    match event.status & 0xF0 {
        0x90 if velocity > 0 => NoteAction::On { pitch, velocity },
        // ⭐ `0x80`（NoteOff）**与** `0x90` 且力度 0（running status）是同一条分支。
        0x90 | 0x80 => NoteAction::Off { pitch },
        _ => NoteAction::NotANote,
    }
}

/// ⭐ **一条 MIDI 事件 ⇒ 引擎事件**（`None` ＝ 不是音符 ⇒ 调用方**计数**，⛔ 不静默丢）。
#[must_use]
pub fn engine_event_for(event: MidiEvent, track: EntityId) -> Option<EngineEvent> {
    match note_action(event) {
        NoteAction::On { pitch, velocity } => Some(EngineEvent::NoteOn {
            track,
            pitch,
            velocity,
        }),
        NoteAction::Off { pitch } => Some(EngineEvent::NoteOff { track, pitch }),
        NoteAction::NotANote => None,
    }
}

/// ⭐ 一次泵的**全部读数**（⚠ 每一条被忽略的消息也要**计数**，⛔ 不静默丢）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LivePumpReport {
    /// 从 [`MidiEventReceiver`] 弹出并**分类过**的 MIDI 事件条数
    /// （`= note_ons + note_offs + ignored`）。
    pub drained: usize,
    /// 映射成 [`EngineEvent::NoteOn`] 的条数。
    pub note_ons: usize,
    /// 映射成 [`EngineEvent::NoteOff`] 的条数（⭐ **含**力度 0 的 `0x9n`）。
    pub note_offs: usize,
    /// ⭐ **非音符**消息条数（CC / 弯音 / 触后 / 实时 / 系统）—— **计数**，⛔ 不静默丢。
    pub ignored: usize,
    /// 真的写进引擎环的事件条数（⛔ 不是"打算写"的条数）。
    pub published: usize,
    /// ⭐ 批量发布**调用次数**（⛔ 不是事件条数）。
    ///
    /// ⚠ 口径：一个非空批 ⇒ +1；⭐ 空批（整批都是非音符消息）**不发布** ⇒ 也 +0。
    /// 结构性判据用它钉住"**批量**发布"（`ring.rs` 的 `bulk_push_calls` 契约）。
    pub publishes: usize,
    /// ⭐ 映射出来却**没写进环**的条数（环满 ⇒ `publish` 返回的条数 < 批长度）。
    ///
    /// ⚠ 这些事件**确实丢了**（已经离开 MIDI 环，回不去）⇒ 因此必须**如实计数**，
    /// ⛔ 不静默丢。
    pub refused: usize,
    /// ⭐ **MIDI 环自己丢的**条数（环满 ⇒ `yeban-midi-io` 侧的共享计数，累计值）。
    pub midi_dropped: u64,
    /// ⭐ **本仓不承载的**消息条数（系统专用 / 可变长 —— 它们在 `yeban-midi-io` 侧就没进环，
    /// 累计值）。⛔ 也不静默丢。
    pub unsupported: u64,
}

impl LivePumpReport {
    /// 映射出来的音符事件总数（＝ `published + refused` 的**应然**值）。
    #[must_use]
    pub const fn mapped(&self) -> usize {
        self.note_ons.saturating_add(self.note_offs)
    }

    /// ⭐ 把另一次泵的读数**并进来**（连续泵的分段读数合账）。
    ///
    /// ⚠ 口径：前缀各计数**相加**；`midi_dropped` / `unsupported` 是**累计读数** ⇒
    /// 取**后一次的**（⛔ 不相加，否则重复计）。
    pub fn merge(&mut self, step: &Self) {
        self.drained = self.drained.saturating_add(step.drained);
        self.note_ons = self.note_ons.saturating_add(step.note_ons);
        self.note_offs = self.note_offs.saturating_add(step.note_offs);
        self.ignored = self.ignored.saturating_add(step.ignored);
        self.published = self.published.saturating_add(step.published);
        self.publishes = self.publishes.saturating_add(step.publishes);
        self.refused = self.refused.saturating_add(step.refused);
        self.midi_dropped = step.midi_dropped;
        self.unsupported = step.unsupported;
    }

    /// 一行可读摘要（CLI / 判据都读它；⭐ 分子/分母明说"写进去几条 / 映射出几条"）。
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "midi-live: drained={} note-ons={} note-offs={} ignored={} published={}/{} \
             publishes={} refused={} midi-dropped={} unsupported={}",
            self.drained,
            self.note_ons,
            self.note_offs,
            self.ignored,
            self.published,
            self.mapped(),
            self.publishes,
            self.refused,
            self.midi_dropped,
            self.unsupported,
        )
    }
}

/// ⭐ 泵的**核心**：搬运**一批**（至多 [`PUMP_CHUNK`] 条）并按到达顺序交给 `publish`。
///
/// ⭐ 为什么核心是**泛型**的：泵的产物是"一串**按到达顺序**的事件"，把它交给谁（真的
/// 引擎环 / 判据的记账闭包）是**调用方**的事 ⇒ 判据因此能**逐条读回**顺序与批边界。
/// ⭐ 产品路径只有一条：[`pump_once`] 把它绑到 [`EngineHost::send_engine_events`]。
///
/// `publish` 的返回值 ＝ **真的收下的条数**（环满 ⇒ 少于批长度 ⇒ 记进
/// [`LivePumpReport::refused`]，⛔ 不假装成功）。
///
/// ⭐ 本函数**零堆分配**（两个定长栈缓冲）、⛔ 不加锁、⛔ 不做 I/O。
#[must_use]
pub fn pump_once_with<F>(
    receiver: &mut MidiEventReceiver,
    track: EntityId,
    mut publish: F,
) -> LivePumpReport
where
    F: FnMut(&[EngineEvent]) -> usize,
{
    // ---- ① 弹出至多 PUMP_CHUNK 条（FIFO ⇒ 顺序就是到达顺序）----
    let mut incoming = [EMPTY_MIDI_EVENT; PUMP_CHUNK];
    let mut popped = 0_usize;
    while popped < PUMP_CHUNK {
        match receiver.pop() {
            Some(event) => {
                incoming[popped] = event;
                popped = popped.saturating_add(1);
            }
            None => break,
        }
    }

    // ---- ② 逐条映射（⭐ 1 条 MIDI ⇒ 至多 1 条引擎事件 ⇒ 缓冲不会溢出）----
    let mut report = LivePumpReport {
        drained: popped,
        ..LivePumpReport::default()
    };
    let mut outgoing = [EngineEvent::IDLE; PUMP_CHUNK];
    let mut mapped = 0_usize;
    for event in &incoming[..popped] {
        match note_action(*event) {
            NoteAction::On { pitch, velocity } => {
                outgoing[mapped] = EngineEvent::NoteOn {
                    track,
                    pitch,
                    velocity,
                };
                mapped = mapped.saturating_add(1);
                report.note_ons = report.note_ons.saturating_add(1);
            }
            NoteAction::Off { pitch } => {
                outgoing[mapped] = EngineEvent::NoteOff { track, pitch };
                mapped = mapped.saturating_add(1);
                report.note_offs = report.note_offs.saturating_add(1);
            }
            // ⭐ 非音符消息：**计数**，⛔ 不静默丢，⛔ 也不进环（引擎的音符路径不认识
            // CC / 弯音 / 触后 —— 发明一条映射才是错的）。
            NoteAction::NotANote => report.ignored = report.ignored.saturating_add(1),
        }
    }

    // ---- ③ 一次批量发布（⛔ 不是一条一次）----
    if mapped > 0 {
        let published = publish(&outgoing[..mapped]);
        report.published = published.min(mapped);
        report.refused = mapped.saturating_sub(report.published);
        report.publishes = 1;
    }

    // ---- ④ 两侧的"丢"都**如实读数**（累计值，⛔ 不静默）----
    report.midi_dropped = receiver.dropped_events();
    report.unsupported = receiver.unsupported_messages();
    report
}

/// ⭐ **产品绑定**：一次泵 ⇒ 既有的 [`EngineHost::send_engine_events`]（UI→引擎 SPSC）。
///
/// ⭐ 连续运行的应用应当**每个量子 / 每个 UI tick 调一次**（每次至多 [`PUMP_CHUNK`] 条
/// ⇒ 控制侧不会被事件洪峰拖住）；⛔ 不要写成"等采集窗口结束再一次性处理" —— 那正是本模块
/// 要补掉的形状。
#[must_use]
pub fn pump_once(
    receiver: &mut MidiEventReceiver,
    track: EntityId,
    host: &mut EngineHost,
) -> LivePumpReport {
    pump_once_with(receiver, track, |batch| host.send_engine_events(batch))
}

/// ⭐ **连续泵到（当下）环空**：反复 [`pump_once`] 直到一次都没弹出。
///
/// ⚠ **终止条件**：环**空**为止 ⇒ 生产者**持续**灌入时本函数**不返回**。因此
/// ⛔ 持续运行的应用用 [`pump_once`]；本函数是"把**现在**积压的处理干净"的入口
/// （判据 / 关停前收尾）。
#[must_use]
pub fn pump_available(
    receiver: &mut MidiEventReceiver,
    track: EntityId,
    host: &mut EngineHost,
) -> LivePumpReport {
    let mut total = LivePumpReport::default();
    loop {
        let step = pump_once(receiver, track, host);
        let drained = step.drained;
        total.merge(&step);
        if drained == 0 {
            return total;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 ⑨（**映射**，纯函数）：⭐ 逐条钉住映射表 —— 尤其 **`0x9n` 力度 0 ⇒ NoteOff**
    /// 这条经典 bug 点。
    #[test]
    fn the_mapping_is_pinned_message_by_message() {
        let track = EntityId::new();
        let event = |status: u8, data1: u8, data2: u8| MidiEvent {
            timestamp_us: 0,
            status,
            data1,
            data2,
        };

        assert_eq!(
            engine_event_for(event(0x90, 60, 100), track),
            Some(EngineEvent::NoteOn {
                track,
                pitch: 60,
                velocity: 100
            }),
            "0x90 且力度 > 0 ⇒ NoteOn"
        );
        assert_eq!(
            engine_event_for(event(0x80, 60, 0), track),
            Some(EngineEvent::NoteOff { track, pitch: 60 }),
            "0x80 ⇒ NoteOff"
        );
        assert_eq!(
            engine_event_for(event(0x90, 60, 0), track),
            Some(EngineEvent::NoteOff { track, pitch: 60 }),
            "⭐ 0x90 且力度 == 0 ⇒ NoteOff（running status 的常见写法）"
        );
        assert_eq!(
            engine_event_for(event(0x90, 60, 0x80), track),
            Some(EngineEvent::NoteOff { track, pitch: 60 }),
            "越界的数据字节按 0x7F 掩码 ⇒ 不被读成幻觉音符"
        );
        for status in [0xA0_u8, 0xB0, 0xC0, 0xD0, 0xE0, 0xF8, 0xFF] {
            assert_eq!(
                engine_event_for(event(status, 7, 100), track),
                None,
                "非音符消息必须不被映射（status={status:#04X}）"
            );
            assert_eq!(note_action(event(status, 7, 100)), NoteAction::NotANote);
        }
        assert_eq!(
            engine_event_for(event(0x9F, 64, 90), track),
            Some(EngineEvent::NoteOn {
                track,
                pitch: 64,
                velocity: 90
            }),
            "通道号（低 4 位）不影响判定"
        );
    }

    /// 判据 ⑩（**顺序**，行为）：⭐ 混合序列（音符开 / CC / 音符开 / 力度 0 开 / 音符关）
    /// ⇒ 泵出一批，**逐条读回**其顺序与到达顺序一致；非音符**计数**。
    #[test]
    fn the_pump_preserves_arrival_order_and_counts_non_notes() {
        let track = EntityId::new();
        let (mut sender, mut receiver) = event_channel(64);
        // ⭐ 与真回调**同一段代码**（`MidiEventSender::handle_message` 是回调体的内核）。
        sender.handle_message(0, &[0x90, 60, 100]);
        sender.handle_message(1_000, &[0xB0, 7, 100]); // CC7 ⇒ 非音符
        sender.handle_message(2_000, &[0x90, 64, 90]);
        sender.handle_message(3_000, &[0x90, 60, 0]); // 力度 0 ⇒ NoteOff
        sender.handle_message(4_000, &[0x80, 64, 0]);
        sender.handle_message(5_000, &[0xF8]); // 实时字节 ⇒ 非音符

        let mut published: Vec<EngineEvent> = Vec::new();
        let report = pump_once_with(&mut receiver, track, |batch| {
            published.extend_from_slice(batch);
            batch.len()
        });

        assert_eq!(
            published,
            vec![
                EngineEvent::NoteOn {
                    track,
                    pitch: 60,
                    velocity: 100
                },
                EngineEvent::NoteOn {
                    track,
                    pitch: 64,
                    velocity: 90
                },
                EngineEvent::NoteOff { track, pitch: 60 },
                EngineEvent::NoteOff { track, pitch: 64 },
            ],
            "⭐ 必须是**到达顺序**（⭐ CC 与实时字节不进这批）"
        );
        assert_eq!(report.drained, 6);
        assert_eq!(report.note_ons, 2);
        assert_eq!(report.note_offs, 2);
        assert_eq!(report.ignored, 2, "CC7 与 0xF8 必须被**计数**");
        assert_eq!(
            report.publishes, 1,
            "⭐ 4 条事件 ⇒ **一次**批量发布（⛔ 不是 4 次）"
        );
        assert_eq!(report.published, 4);
        assert_eq!(report.refused, 0);
        assert_eq!(receiver.pending(), 0, "一次泵把这一批全部弹出");
        assert!(
            report
                .summary()
                .contains("drained=6 note-ons=2 note-offs=2 ignored=2"),
            "{}",
            report.summary()
        );
    }

    /// 判据 ⑪（**有界批 + 两边的"丢"都计数**，行为）：
    /// ① 一个 MIDI 小环塞不下时，**接收侧读数**如实增长（`midi_dropped`）；
    /// ② 灌进**比引擎环容量更多**的音符事件 ⇒ 发布**分批**（`publishes` ＝ ⌈N/128⌉，
    ///    ⛔ 不是 N）且"写进去的 + 被拒的 ＝ 映射总数"（⛔ 不静默丢）。
    #[test]
    fn the_pump_batches_and_counts_everything_it_cannot_carry() {
        let track = EntityId::new();

        // ---- ① MIDI 侧的丢弃：环容量 4，灌 10 条 ⇒ 接收侧读数 = 6 ----
        let (mut sender, mut receiver) = event_channel(4);
        for _ in 0..10 {
            sender.handle_message(0, &[0x90, 60, 100]);
        }
        assert_eq!(sender.dropped_events(), 6, "生产侧读数");
        assert_eq!(receiver.dropped_events(), 6, "⭐ 接收侧读到**同一个事实**");
        let mut host = EngineHost::new();
        host.reload(&crate::bridge::demo_project(), 0)
            .expect("⭐ 必须先建引擎：没有引擎时 `send_engine_events` 恒返回 0");
        let report = pump_once(&mut receiver, track, &mut host);
        assert_eq!(report.drained, 4, "环里只装得下 4 条");
        assert_eq!(report.midi_dropped, 6, "⭐ 环满丢的 6 条必须出现在报告里");
        assert_eq!(report.note_ons, 4);
        assert_eq!(report.published, 4, "环没满 ⇒ 4 条全部进引擎环");

        // ---- ② 引擎侧的拒绝与批量：300 条音符 ⇒ ⌈300/128⌉ = 3 次发布 ----
        let mut host = EngineHost::new();
        host.reload(&crate::bridge::demo_project(), 0)
            .expect("建引擎（本段只判'发布'这一层：泵与环）");
        let (mut sender, mut receiver) = event_channel(512);
        for _ in 0..300 {
            sender.handle_message(0, &[0x90, 60, 100]);
        }
        let report = pump_available(&mut receiver, track, &mut host);
        assert_eq!(report.drained, 300);
        assert_eq!(report.note_ons, 300);
        assert_eq!(report.ignored, 0);
        assert_eq!(
            report.publishes, 3,
            "⭐ 300 条 ⇒ 3 次批量发布（⌈300/{}⌉），⛔ 不是 300 次",
            PUMP_CHUNK
        );
        assert_eq!(
            report.published + report.refused,
            300,
            "⭐ 写进去的 + 被环拒的 ⇒ 必须是 300（⛔ 没有第三条去向）"
        );
        assert!(
            report.refused > 0,
            "⚠ 本判据**钉住今天的事实**：引擎环容量（`engine_host.rs` 的 EVENT_CAPACITY = 256）\
             小于 300 ⇒ 超出的必须被**计数**（refused={}），⛔ 不静默丢",
            report.refused
        );
        assert_eq!(receiver.pending(), 0);
        println!("[midi-live] 300 条 ⇒ {}", report.summary());
    }
}
