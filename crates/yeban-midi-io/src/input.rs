//! `midir` 真实接线：端口枚举 → 定长事件 → 预分配无锁环 → 960 PPQ Tick [`ADR-0006`]。
//!
//! # 本模块的语义来源
//!
//! ⭐ **环的语义照抄 `yeban-engine` 的 `ring.rs::event_channel`／`record_channel`**
//! （[`ARCH-RT-001`]）：
//!
//! 1. ⭐ **整事件原子** —— 推送失败时**不留下半条事件**（⭐ `rtrb` 的 `push` 对单个元素
//!    是原子的 ⇒ 本模块**只**按"一个 `MidiEvent`"为单位推送），⭐ 且该次失败
//!    **不计入**读者可见的事件流；
//! 2. ⭐ **环满 ⇒ 按事件计数丢弃 ＋ ⛔ 永不阻塞** —— 失败次数记在
//!    [`MidiEventSender::dropped_events`] 上，⭐ 推送路径**立刻返回**；
//! 3. ⭐ **推送路径零分配／零锁** —— 环在**构造期**一次性分配（`rtrb::RingBuffer::new`），
//!    推送/弹出**只碰索引与 `MaybeUninit` 槽位**。
//!
//! ⚠ ⭐ **本模块刻意不依赖 `yeban-engine`**：那会把 `cpal`／声卡后端拖进本 crate，
//! 也会触犯依赖方向守卫 ⇒ ⭐ **复用语义、⛔ 不复用 crate**。
//!
//! ⚠ ⭐ **已登记的断点（`lib.rs` 的四条清单在本模块的落点）**：
//!
//! 1. ⭐ 事件时间戳**取自 `midir` 回调的第一个参数**（⛔ `avoid_timestamping` 保持关闭）；
//! 2. ⭐ 回调内**零分配／零 I/O／不加锁**（⭐ 用本模块的零分配窗口判据判）；
//! 3. ⭐ 换算基准 ＝ **调用方显式给出的 `origin_us`**（⭐ 记在 [`MidiInputSession::origin_us`]），
//!    ⛔ 不是回调到达时刻；
//! 4. ⭐ **`RSK-22` 的抖动量化口径尚未裁定** ⇒ 本模块只给**临时**读数口径
//!    （[`JitterReading`]：p50／p99／max ＋ 样本量），⭐ **标注 `待裁`**，⛔ 不声称 5 Tick 达标。
//!
//! [`ADR-0006`]: ../../docs/adr/ADR-0006-hardware-midi-input-via-midir.md
//! [`ARCH-RT-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md

use crate::ticks_from_origin_us;

/// ⭐ 环的默认容量（⭐ 事件条数，⭐ 与 `ring.rs::DEFAULT_EVENT_CAPACITY` 同量级）。
pub const DEFAULT_EVENT_CAPACITY: usize = 1024;

/// ⭐ 一个 MIDI 输入端口（⭐ **非实时路径** ⇒ 允许分配，[`ADR-0006`] D3）。
///
/// [`ADR-0006`]: ../../docs/adr/ADR-0006-hardware-midi-input-via-midir.md
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiPortInfo {
    /// `midir` 端口列表里的下标（⭐ 连接时用它定位端口）。
    pub index: usize,
    /// 端口名（⭐ 由 `midir` 给出，⭐ 已尽力转成 `String`）。
    pub name: String,
}

/// ⭐ **定长**的 MIDI 事件（⭐ `Copy` ⇒ 环里没有所有权、⭐ 无 `Drop`）。
///
/// ⚠ ⭐ **通道口径（登记项）**：`status`／`data1`／`data2` 是**逐字节**的定长三字节。
/// ⭐ **系统实时（`0xF8..=0xFF`）** 单字节消息按"`data1 = data2 = 0`"承载；
/// ⚠ ⭐ **系统专用（`0xF0`）与其它多字节/可变长消息不进本结构** ——
/// ⭐ 由 [`MidiEventSender::unsupported_messages`] **计数登记**，⛔ **不静默丢弃**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiEvent {
    /// ⭐ `midir` 的**原始**时间戳（微秒，⭐ ⛔ 不是本机 `Instant`）。
    pub timestamp_us: u64,
    /// 状态字节（⭐ 含通道号的低 4 位）。
    pub status: u8,
    /// 第一个数据字节（⭐ 无数据字节时为 0）。
    pub data1: u8,
    /// 第二个数据字节（⭐ 无第二个数据字节时为 0）。
    pub data2: u8,
}

impl MidiEvent {
    /// ⭐ 由**状态字节**判断这条消息有多少个数据字节（⭐ 定长三字节的填充规则）。
    ///
    /// 返回 `None` ⇒ ⭐ **本模块不承载**这种消息（⭐ 由调用方计数登记）。
    #[must_use]
    pub const fn data_bytes_expected(status: u8) -> Option<usize> {
        match status {
            // 通道声音消息：⭐ 0x8n/0x9n/0xAn/0xBn/0xEn 两个数据字节。
            0x80..=0xBF | 0xE0..=0xEF => Some(2),
            // 通道消息：⭐ 0xCn/0xDn 一个数据字节。
            0xC0..=0xDF => Some(1),
            // 系统实时：⭐ 0xF8..=0xFF 没有数据字节。
            0xF8..=0xFF => Some(0),
            // ⭐ 系统专用（0xF0/0xF7）与系统共同（0xF1..=0xF6）⇒ **不承载**。
            _ => None,
        }
    }

    /// ⭐ 由一条**字节消息**组装定长事件（⭐ 纯函数、⭐ **零分配**）。
    ///
    /// ⭐ `None` ⇒ **不承载**（⭐ 长度与状态字节不符，或状态字节本就不承载）。
    #[must_use]
    pub fn from_message(timestamp_us: u64, message: &[u8]) -> Option<Self> {
        let status = *message.first()?;
        let expected = Self::data_bytes_expected(status)?;
        if message.len() != expected + 1 {
            return None;
        }
        Some(Self {
            timestamp_us,
            status,
            data1: *message.get(1).unwrap_or(&0),
            data2: *message.get(2).unwrap_or(&0),
        })
    }

    /// ⭐ 换算成**相对会话起点**的 960 PPQ Tick（[`MODEL-AST-001`]）。
    ///
    /// [`MODEL-AST-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
    #[must_use]
    pub fn ticks_from(&self, origin_us: u64) -> u64 {
        ticks_from_origin_us(self.timestamp_us, origin_us)
    }
}

/// ⭐ 生产者侧（⭐ **`midir` 回调线程**持有；⭐ 推送路径零分配／零锁／永不阻塞）。
pub struct MidiEventSender {
    producer: rtrb::Producer<MidiEvent>,
    dropped_events: u64,
    unsupported_messages: u64,
}

impl MidiEventSender {
    /// ⭐ 环容量（⭐ 事件条数）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.producer.buffer().capacity()
    }

    /// ⭐ 还能放几条（⭐ 读数，⭐ 非实时路径用）。
    #[must_use]
    pub fn free_slots(&self) -> usize {
        self.producer.slots()
    }

    /// ⭐ **按事件推送**（⭐ 整事件原子 ⇒ ⛔ 不留半条；⭐ 环满 ⇒ 计数 +1 并**立刻**返回 `false`）。
    ///
    /// [`ARCH-RT-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
    pub fn push(&mut self, event: MidiEvent) -> bool {
        match self.producer.push(event) {
            Ok(()) => true,
            Err(_full) => {
                // ⭐ 环满：⭐ 事件**整体**没进去 ⇒ ⭐ 按**事件**计数丢弃，⭐ ⛔ 不阻塞、⛔ 不分配。
                self.dropped_events = self.dropped_events.saturating_add(1);
                false
            }
        }
    }

    /// ⭐ **被丢弃的事件条数**（⭐ 单位 = 事件，⛔ 不是字节、⛔ 不是"半条"）。
    #[must_use]
    pub const fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// ⭐ **不承载的消息条数**（⭐ 系统专用/可变长 ⇒ ⭐ 登记，⛔ 不静默丢弃）。
    #[must_use]
    pub const fn unsupported_messages(&self) -> u64 {
        self.unsupported_messages
    }

    /// ⭐ **模拟回调体**：⭐ 由 `(timestamp_us, &[u8])` 推进环 —— 与真实 `midir` 回调
    /// **走同一段代码**（⭐ 本函数就是回调体的内核，⭐ 真回调只多一层 `timestamp_us` 绑定）。
    ///
    /// ⭐ **本函数必须保持零分配／零 I/O／零锁**（[`ARCH-RT-001`]）。
    ///
    /// [`ARCH-RT-001`]: ../../docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md
    pub fn handle_message(&mut self, timestamp_us: u64, message: &[u8]) {
        match MidiEvent::from_message(timestamp_us, message) {
            Some(event) => {
                let _accepted = self.push(event);
            }
            None => {
                self.unsupported_messages = self.unsupported_messages.saturating_add(1);
            }
        }
    }
}

/// ⭐ 消费者侧（⭐ **控制侧线程**持有；⭐ 非实时路径）。
pub struct MidiEventReceiver {
    consumer: rtrb::Consumer<MidiEvent>,
}

impl MidiEventReceiver {
    /// ⭐ 环容量（⭐ 事件条数）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.consumer.buffer().capacity()
    }

    /// ⭐ 当前可弹出几条。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// ⭐ 弹出**一条完整**事件（⭐ 整事件原子 ⇒ ⛔ 看不到半条）。
    pub fn pop(&mut self) -> Option<MidiEvent> {
        self.consumer.pop().ok()
    }
}

/// ⭐ 建一对（生产者，消费者）—— ⭐ **环在此一次性分配**（⭐ 之后推送/弹出 ⛔ 不再分配）。
#[must_use]
pub fn event_channel(capacity: usize) -> (MidiEventSender, MidiEventReceiver) {
    let (producer, consumer) = rtrb::RingBuffer::new(capacity);
    (
        MidiEventSender {
            producer,
            dropped_events: 0,
            unsupported_messages: 0,
        },
        MidiEventReceiver { consumer },
    )
}

/// ⭐ **`RSK-22` 的抖动读数**（⭐ ⚠️ **临时口径，待裁**）。
///
/// ⚠ ⭐ 人类裁决：⭐ `RSK-22` 的**量化口径留待实现期再裁** ⇒ ⭐ 本结构只把
/// **p50／p99／max ＋ 样本量** 摆出来，⛔ **不得**据此声称"5 Tick 那条线已达标"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JitterReading {
    /// 样本量（⭐ 单位 = 样本条数）。
    pub n: usize,
    /// 中位偏差（⭐ 单位 = Tick）。
    pub p50: u64,
    /// 99 分位偏差（⭐ 单位 = Tick）。
    pub p99: u64,
    /// 最大偏差（⭐ 单位 = Tick）。
    pub max: u64,
}

/// ⭐ 由一串**偏差读数**（⭐ 单位 = Tick）算 [`JitterReading`]（⭐ 纯函数、⭐ 可无声卡判定）。
///
/// ⭐ 口径：⭐ 对**排序后的偏差**取 `p50 = 第 ⌈n/2⌉ 个`、`p99 = 第 ⌈0.99n⌉ 个`、
/// `max = 最后一个`；⭐ `n == 0` ⇒ 全 0（⭐ ⛔ 不 panic）。
#[must_use]
pub fn jitter_reading(deviations_ticks: &[u64]) -> JitterReading {
    if deviations_ticks.is_empty() {
        return JitterReading {
            n: 0,
            p50: 0,
            p99: 0,
            max: 0,
        };
    }
    let mut sorted = deviations_ticks.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    let at = |numerator: usize, denominator: usize| -> u64 {
        let rank = (n * numerator).div_ceil(denominator).max(1);
        sorted[rank.min(n) - 1]
    };
    JitterReading {
        n,
        p50: at(1, 2),
        p99: at(99, 100),
        max: sorted[n - 1],
    }
}

/// ⭐ 输入侧的失败面（⭐ ⛔ 不 panic：⭐ 无声卡/无端口 ⇒ 空 `Vec`，⭐ 见 [`enumerate_input_ports`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MidiInputError {
    /// ⭐ `midir` 自身初始化失败（⭐ 例：⭐ 宿主不可用）。
    HostUnavailable(String),
    /// ⭐ 端口下标越界（⭐ 例：⭐ 枚举之后设备被拔掉）。
    NoSuchPort {
        /// 请求的下标。
        index: usize,
    },
    /// ⭐ 连接失败（⭐ `midir` 的原因）。
    ConnectFailed(String),
}

impl core::fmt::Display for MidiInputError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::HostUnavailable(reason) => write!(f, "MIDI 宿主不可用: {reason}"),
            Self::NoSuchPort { index } => write!(f, "没有下标为 {index} 的 MIDI 输入端口"),
            Self::ConnectFailed(reason) => write!(f, "连接 MIDI 输入端口失败: {reason}"),
        }
    }
}

impl std::error::Error for MidiInputError {}

/// ⭐ **枚举输入端口**（⭐ 非实时路径 ⇒ 允许分配）。
///
/// ⚠ ⭐ **零端口（⭐ 无声卡／无 MIDI 设备）⇒ 返回空 `Vec`、⭐ ⛔ 不 panic、⛔ 不阻塞**
/// （⭐ 照输出侧 `enumerate_output_devices` 的惯例）。
///
/// # Errors
///
/// ⭐ 只在 `midir` 宿主**自身**不可用时返回 [`MidiInputError::HostUnavailable`]。
#[cfg(feature = "midi-input")]
pub fn enumerate_input_ports() -> Result<Vec<MidiPortInfo>, MidiInputError> {
    let host = midir::MidiInput::new("yeban-midi-io")
        .map_err(|error| MidiInputError::HostUnavailable(error.to_string()))?;
    // ⭐ 零端口 ⇒ 空 Vec（⛔ 不报错、⛔ 不 panic）。
    Ok(host
        .ports()
        .iter()
        .enumerate()
        .map(|(index, port)| MidiPortInfo {
            index,
            name: host
                .port_name(port)
                .unwrap_or_else(|_| "<无法读取端口名>".to_owned()),
        })
        .collect())
}

/// ⭐ 一次**已连接**的输入会话（⭐ 持有 `midir` 的连接 ⇒ 掉线即关闭端口）。
///
/// ⚠ ⭐ **生命周期断点**：⭐ `midir` 的 `connect → 真回调线程` 在**本机与 CI 都无法覆盖**
/// （⭐ CI 容器无 MIDI 硬件）⇒ ⭐ 本结构只在**有声卡的机器**上才会真正跑到回调。
#[cfg(feature = "midi-input")]
pub struct MidiInputSession {
    origin_us: u64,
    // ⭐ **RAII 句柄**：⭐ 只为"⭐ 会话活着 ⇒ 端口保持打开"而持有
    // （⭐ `midir` 在连接被 drop 时关闭端口）⇒ ⭐ **刻意不读**它。
    #[expect(
        dead_code,
        reason = "RAII 句柄：只为保持端口打开而持有，本 crate 刻意不读它"
    )]
    connection: midir::MidiInputConnection<()>,
    receiver: MidiEventReceiver,
}

#[cfg(feature = "midi-input")]
impl MidiInputSession {
    /// ⭐ **显式记录**的换算基准（微秒）—— ⭐ `midir` 原始时间戳的会话起点。
    #[must_use]
    pub const fn origin_us(&self) -> u64 {
        self.origin_us
    }

    /// ⭐ 消费者侧（⭐ 控制侧线程用它排空事件）。
    #[must_use]
    pub fn receiver(&mut self) -> &mut MidiEventReceiver {
        &mut self.receiver
    }

    /// ⭐ 弹出**一条完整**事件（⭐ 便捷转发）。
    pub fn pop(&mut self) -> Option<MidiEvent> {
        self.receiver.pop()
    }
}

/// ⭐ **连接一个输入端口**（⭐ feature `midi-input` 之下）。
///
/// ⚠ ⭐ **`origin_us` 必须由调用方显式给出**（[`ADR-0006`] D4）：⭐ 典型做法是
/// **连接时取一次 `midir` 时间戳**（⭐ 例：⭐ 在控制侧的第一次 `Instant` 采样对应的
/// `midir` 时间戳）作为会话起点，⭐ 并把它记进 [`MidiInputSession::origin_us`]。
/// ⛔ **本函数不接受"当前时刻"当基准** —— 那会把线程调度延迟算进音乐时间
/// （⭐ 那正是 `RSK-22` 要量的抖动）。
///
/// # Errors
///
/// ⭐ 宿主不可用 ⇒ [`MidiInputError::HostUnavailable`]；
/// ⭐ 端口下标越界 ⇒ [`MidiInputError::NoSuchPort`]；
/// ⭐ `connect` 失败 ⇒ [`MidiInputError::ConnectFailed`]。
///
/// [`ADR-0006`]: ../../docs/adr/ADR-0006-hardware-midi-input-via-midir.md
#[cfg(feature = "midi-input")]
pub fn open_input(
    index: usize,
    origin_us: u64,
    capacity: usize,
) -> Result<MidiInputSession, MidiInputError> {
    let host = midir::MidiInput::new("yeban-midi-io")
        .map_err(|error| MidiInputError::HostUnavailable(error.to_string()))?;
    let ports = host.ports();
    let port = ports
        .get(index)
        .ok_or(MidiInputError::NoSuchPort { index })?
        .clone();
    let (mut sender, receiver) = event_channel(capacity);
    // ⭐ 回调体与 `MidiEventSender::handle_message`（判据里判的那个）**是同一段代码**
    // ⇒ ⭐ 本机判据覆盖的就是真回调执行的那条路径（⭐ 只多一层 `midir` 的 `timestamp_us` 绑定）。
    let connection = host
        .connect(
            &port,
            "yeban-midi-io",
            move |timestamp_us, message, _| sender.handle_message(timestamp_us, message),
            (),
        )
        .map_err(|error| MidiInputError::ConnectFailed(error.to_string()))?;
    Ok(MidiInputSession {
        origin_us,
        connection,
        receiver,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 判据 ①：⭐ 环是**整事件原子**的 —— ⭐ 放进去 N 条、弹出来必须**逐条完整**
    /// （⭐ 字段全等），⭐ ⛔ 不会出现"半条"。
    #[test]
    fn events_come_out_whole_and_exactly_as_pushed() {
        let (mut sender, mut receiver) = event_channel(8);
        let pushed = [
            MidiEvent {
                timestamp_us: 10,
                status: 0x90,
                data1: 60,
                data2: 100,
            },
            MidiEvent {
                timestamp_us: 20,
                status: 0x80,
                data1: 60,
                data2: 0,
            },
            MidiEvent {
                timestamp_us: 30,
                status: 0xB0,
                data1: 7,
                data2: 127,
            },
        ];
        for event in pushed {
            assert!(sender.push(event), "环未满 ⇒ 必须接收");
        }
        assert_eq!(sender.dropped_events(), 0);
        let mut popped = Vec::new();
        while let Some(event) = receiver.pop() {
            popped.push(event);
        }
        assert_eq!(popped, pushed, "逐条必须与推入的完全相同");
    }

    /// 判据 ②：⭐ **环满 ⇒ 按事件计数丢弃 ＋ 立刻返回**（⭐ ⛔ 不阻塞、⭐ ⛔ 不留半条）。
    #[test]
    fn a_full_ring_drops_whole_events_and_never_blocks() {
        let (mut sender, mut receiver) = event_channel(2);
        let event = MidiEvent {
            timestamp_us: 1,
            status: 0x90,
            data1: 1,
            data2: 2,
        };
        assert!(sender.push(event));
        assert!(sender.push(event));
        // ⭐ 这两次必然失败：⭐ 容量是 2。
        assert!(!sender.push(event));
        assert!(!sender.push(event));
        assert_eq!(sender.dropped_events(), 2, "丢弃必须**按事件**计数");
        // ⭐ 读者侧**仍然只看到 2 条完整事件**（⛔ 没有半条、⛔ 没有空洞）。
        assert_eq!(receiver.pending(), 2);
        let first = receiver.pop().expect("完整事件");
        let second = receiver.pop().expect("完整事件");
        assert_eq!(first, event);
        assert_eq!(second, event);
        assert!(receiver.pop().is_none(), "读完即空");
    }

    /// 判据 ③：⭐ **模拟回调体**：⭐ 由字节消息组装定长事件并推进环（⭐ 与真回调同段代码）。
    #[test]
    fn the_simulated_callback_body_pushes_expected_events() {
        let (mut sender, mut receiver) = event_channel(8);
        sender.handle_message(1_000, &[0x90, 60, 100]); // ⭐ NoteOn 三字节
        sender.handle_message(2_000, &[0xC0, 5]); // ⭐ ProgramChange 两字节
        sender.handle_message(3_000, &[0xF8]); // ⭐ 系统实时 一字节
        assert_eq!(receiver.pending(), 3);
        assert_eq!(
            receiver.pop(),
            Some(MidiEvent {
                timestamp_us: 1_000,
                status: 0x90,
                data1: 60,
                data2: 100
            })
        );
        assert_eq!(
            receiver.pop(),
            Some(MidiEvent {
                timestamp_us: 2_000,
                status: 0xC0,
                data1: 5,
                data2: 0
            }),
            "一个数据字节的消息 ⇒ data2 补 0"
        );
        assert_eq!(
            receiver.pop(),
            Some(MidiEvent {
                timestamp_us: 3_000,
                status: 0xF8,
                data1: 0,
                data2: 0
            }),
            "系统实时 ⇒ 没有数据字节"
        );
    }

    /// 判据 ④（⭐ **登记项**）：⭐ **不承载**的消息被**计数登记**，⭐ ⛔ 不静默丢弃。
    #[test]
    fn unsupported_messages_are_counted_not_silently_dropped() {
        let (mut sender, _receiver) = event_channel(8);
        // ⭐ 系统专用（0xF0）：本模块不承载。
        sender.handle_message(1, &[0xF0, 0x7E, 0x00, 0xF7]);
        // ⭐ 系统共同（0xF1）：本模块不承载。
        sender.handle_message(2, &[0xF1, 0x00]);
        // ⭐ 长度与状态字节不符：也不承载。
        sender.handle_message(3, &[0x90, 60]);
        assert_eq!(sender.unsupported_messages(), 3, "不承载的消息必须被计数");
        assert_eq!(sender.dropped_events(), 0, "它们不是环满丢弃");
    }

    /// 判据 ⑤：⭐ 模拟事件序列 ⇒ Tick **单调不减**，⭐ 且**真的在推进**（⭐ 反向对照）。
    #[test]
    fn simulated_timestamps_map_to_monotonic_ticks() {
        let origin = 1_000_000;
        let (mut sender, mut receiver) = event_channel(64);
        for step in 0..16u64 {
            // ⭐ 每 1/16 拍（62.5 ms）一条，⭐ 全部用 `midir` 的原始时间戳（⭐ 不用 Instant）。
            sender.handle_message(origin + step * 62_500, &[0x90, 60, 100]);
        }
        let ticks: Vec<u64> = core::iter::from_fn(|| receiver.pop())
            .map(|event| event.ticks_from(origin))
            .collect();
        assert_eq!(ticks.len(), 16, "16 条都必须能被读到");
        assert!(
            ticks.windows(2).all(|pair| pair[0] <= pair[1]),
            "Tick 必须单调不减：{ticks:?}"
        );
        assert!(ticks[15] > 0, "序列必须真的推进：{ticks:?}");
        assert_eq!(ticks[15], 900, "15 × 60 Tick（每 62.5 ms = 1/16 拍）");
    }

    /// 判据 ⑥：⭐ `MidiEvent` 是 **`Copy` 的定长结构**（⭐ 环里没有所有权／没有 `Drop`）。
    #[test]
    fn the_event_is_copy_and_fixed_size() {
        fn assert_copy<T: Copy>() {}
        assert_copy::<MidiEvent>();
        assert_eq!(
            core::mem::size_of::<MidiEvent>(),
            8 + 1 + 1 + 1 + 5, // ⭐ u64 + 三个 u8 + 对齐填充
            "定长事件的大小必须被钉住（⭐ 改动即红）"
        );
    }

    /// 判据 ⑦（⭐ **R253① 的形态**）：⭐ 状态字节 ⇒ 期望数据字节数 的**双向**表 ——
    /// ⭐ 每个承载的族恰好一条，⭐ ⛔ 不许有"漏族的通配"。
    #[test]
    fn the_status_byte_table_is_two_way_complete() {
        let mut expected: Vec<(u8, Option<usize>)> = Vec::new();
        for status in 0x80u8..=0xFF {
            expected.push((status, MidiEvent::data_bytes_expected(status)));
        }
        assert_eq!(expected.len(), 128, "全部 128 个状态字节都要表态");
        let two = expected
            .iter()
            .filter(|(_, n)| *n == Some(2))
            .map(|(s, _)| *s)
            .collect::<Vec<_>>();
        let one = expected
            .iter()
            .filter(|(_, n)| *n == Some(1))
            .map(|(s, _)| *s)
            .collect::<Vec<_>>();
        let zero = expected
            .iter()
            .filter(|(_, n)| *n == Some(0))
            .map(|(s, _)| *s)
            .collect::<Vec<_>>();
        let none = expected
            .iter()
            .filter(|(_, n)| n.is_none())
            .map(|(s, _)| *s)
            .collect::<Vec<_>>();
        assert_eq!(two.len() + one.len() + zero.len() + none.len(), 128);
        // ⭐ 逐族推导（⭐ 我第一版的期望值 `(64, 32, 8, 24)` 是**我自己算错**的：
        // ⭐ 两个数据字节的族是 `0x80..=0xBF`（**64** 个）＋ `0xE0..=0xEF`（**16** 个）
        // = **80**；⭐ 不承载的族只有 `0xF0..=0xF7`（**8** 个））：
        // ⭐ 80 + 32 + 8 + 8 = 128。
        assert_eq!(
            (two.len(), one.len(), zero.len(), none.len()),
            (80, 32, 8, 8),
            "两字节族 0x80..=0xBF(64) + 0xE0..=0xEF(16) = 80；一字节 0xC0..=0xDF = 32；\
             实时 0xF8..=0xFF = 8；不承载 0xF0..=0xF7 = 8"
        );
    }

    /// 判据 ⑧（⭐ `RSK-22` **临时**口径）：⭐ p50／p99／max ＋ 样本量，⭐ 且**空集不 panic**。
    ///
    /// ⚠ ⭐ 本判据**只钉读数口径**，⛔ **不**声称"5 Tick 那条线已达标"（⭐ 待裁）。
    #[test]
    fn the_provisional_jitter_reading_is_defined_and_does_not_panic_on_empty() {
        assert_eq!(
            jitter_reading(&[]),
            JitterReading {
                n: 0,
                p50: 0,
                p99: 0,
                max: 0
            }
        );
        // ⭐ 100 个样本：⭐ 偏差 = 0..99 Tick。
        let samples: Vec<u64> = (0..100).collect();
        let reading = jitter_reading(&samples);
        assert_eq!(reading.n, 100);
        assert_eq!(reading.p50, 49, "第 ⌈100/2⌉ = 50 个 ⇒ 下标 49");
        assert_eq!(reading.p99, 98, "第 ⌈99⌉ = 99 个 ⇒ 下标 98");
        assert_eq!(reading.max, 99);
        // ⭐ 口径是**序统计量**⇒ 与输入顺序无关（⭐ 反向对照）。
        let mut shuffled = samples.clone();
        shuffled.reverse();
        assert_eq!(jitter_reading(&shuffled), reading);
    }

    /// 判据 ⑨：⭐ **端口枚举的零端口路径**：⭐ 调用必须返回 `Result`、⭐ ⛔ 不 panic。
    ///
    /// ⚠ ⭐ **不断言个数**（⭐ 本机可能**有** MIDI 设备也可能没有 ⇒ 个数不是判据）。
    #[cfg(feature = "midi-input")]
    #[test]
    fn port_enumeration_returns_a_result_and_never_panics() {
        match enumerate_input_ports() {
            Ok(ports) => {
                for port in &ports {
                    assert!(!port.name.is_empty(), "端口名尽力转换 ⇒ 非空；{port:?}");
                }
                eprintln!(
                    "DIAGNOSTIC enumerate_input_ports: 本机端口数={} (⛔ 不是判据)",
                    ports.len()
                );
            }
            Err(error) => {
                // ⭐ 宿主不可用也是**合法**结果（⭐ ⛔ 不 panic）。
                eprintln!("DIAGNOSTIC enumerate_input_ports: 宿主不可用={error}");
            }
        }
    }

    /// 判据 ⑩（⭐ **`RSK-22` 临时读数**）：⭐ 用**模拟**事件序列量"⭐ 相对名义网格的 Tick 偏差"，
    /// ⭐ 只**打印**读数（p50／p99／max ＋ 样本量 N），⭐ **不作为达标断据**。
    ///
    /// ⚠ ⭐ 人类裁决：⭐ `RSK-22` 的口径**留待实现期再裁** ⇒ ⭐ 本判据**只钉口径与可读性**，
    /// ⛔ **不**声称"⭐ 抖动 > 5 Ticks 不可接受"那条线已达标（⭐ 也 ⛔ 不声称未达标）。
    #[test]
    fn a_provisional_jitter_reading_is_printed_for_rsk22() {
        let origin = 1_000_000_u64;
        // ⭐ 名义网格：⭐ 每 62.5 ms（1/16 拍 = 60 Tick）一条 ⇒ 1000 条事件。
        let nominal_step_us = 62_500_u64;
        let mut state = 0x1234_5678_u64;
        let (mut sender, mut receiver) = event_channel(4096);
        for step in 0..1000_u64 {
            // ⭐ 线性同余（⛔ 不引 `rand` 依赖）：⭐ 抖动落 0..=4 ms（⭐ 即 0..≈4 Tick）。
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1);
            let jitter_us = (state >> 33) % 4_001;
            sender.handle_message(
                origin + step * nominal_step_us + jitter_us,
                &[0x90, 60, 100],
            );
        }
        let deviations: Vec<u64> = core::iter::from_fn(|| receiver.pop())
            .enumerate()
            .map(|(step, event)| {
                let nominal = origin + step as u64 * nominal_step_us;
                // ⭐ 偏差用**同一个**换算函数 ⇒ 口径一致（⭐ 单位 = Tick）。
                ticks_from_origin_us(origin + event.timestamp_us.abs_diff(nominal), origin)
            })
            .collect();
        let reading = jitter_reading(&deviations);
        eprintln!(
            "DIAGNOSTIC RSK-22（临时口径，★待裁★）: n={} p50={} p99={} max={} 单位=Tick；\
             负载=cargo test 单进程本机、无并发重活",
            reading.n, reading.p50, reading.p99, reading.max
        );
        assert_eq!(reading.n, 1000, "样本量必须是 1000（⭐ 读数才有分母）");
        assert!(
            reading.p50 <= reading.p99 && reading.p99 <= reading.max,
            "序统计量必须有序：{reading:?}"
        );
    }
}
