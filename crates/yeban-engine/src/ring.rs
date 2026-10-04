//! UI/模型线程 → 音频线程的**批量**无锁 SPSC 事件通道。
//! [ARCH-RT-001, ROAD-M2-007]
//!
//! ## 为什么必须"批量"
//!
//! 逐样本调用 SPSC 的 `push`/`pop` 有几个致命问题：
//!
//! 1. 每次调用都要读对方的原子索引（`Producer` 读 consumer 的缓存位置），
//!    缓存行在两核之间来回争用（false sharing / cache ping-pong）；
//! 2. 每样本一次分支判断"队列空/满"，在实时回调里就是每样本一条不可预测分支；
//! 3. 环形缓冲的翻转点被切成逐样本判断，编译器无法展开。
//!
//! 因此 [ARCH-RT-001] 与 [ROAD-M2-007] 明确要求使用 `rtrb` 的**批量 API**
//! （`Producer::write_chunk` / `push_partial_slice` 与
//! `Consumer::read_chunk` / `pop_partial_slice`），配合**栈上定长临时缓冲**。
//!
//! ## 本模块的批量契约（可机械检验）
//!
//! 每个块边界上，音频线程**只调用一次**批量出队 API：
//!
//! ```text
//! pub fn drain_with(&mut self, scratch, f) -> usize
//!   └─ 恰好 1 次 consumer.pop_partial_slice(scratch)
//! ```
//!
//! 为了让这条契约可断言而不是"靠读代码相信"，[`EventReceiver`] 记录
//! [`bulk_pop_calls`](EventReceiver::bulk_pop_calls)；测试断言"N 次块处理 == N 次批量调用"。
//! 计数器是两个 `u64`，只被音频线程写、只被所有者读，**不是原子量、不是锁**。
//!
//! ## 队列满/空时的行为
//!
//! - 生产侧（UI/模型线程）：`publish` 只写入装得下的部分，返回实际写入条数。
//!   **绝不阻塞、绝不扩容**（扩容 = 分配 = 红线 7）；
//! - 消费侧（音频线程）：`drain_with` 把当前可读的全部批量取出。少收到几个参数事件的
//!   后果是"下一块生效"，而不是爆音或阻塞。
//!
//! 本机不做耗时断言（M2 上墙钟抖动大，脆弱）：判据落在**结构性**的"每块一次批量 API"
//! 上，见 `mod tests` 与 `docs/ledger/engine-rt-notes.md` §3 判据 (d)。

use rtrb::{Consumer, Producer, RingBuffer};
use yeban_model::EntityId;

/// 事件通道的默认容量（条）。够覆盖一整个 60Hz UI 帧的参数洪峰。
pub const DEFAULT_EVENT_CAPACITY: usize = 1024;

/// 音频线程侧建议的栈上临时事件缓冲长度。
///
/// 与 [`crate::block::DEFAULT_BLOCK_FRAMES`] 无关：事件是"每块几条"的量级，
/// 128 已经远超实际（一个块最多几十个参数变化）。
pub const SCRATCH_EVENTS: usize = 128;

/// 参数地址：实体 + 槽位。
///
/// **故意不用字符串**：音频线程做字符串比较意味着指针追踪与不确定的耗时。
/// 槽位由模型层/设备层在发布快照时映射为稳定索引。
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ParamAddress {
    /// 承载参数的实体（音轨 / 设备）。
    pub entity: EntityId,
    /// 该实体内部的参数槽位。
    pub slot: u16,
}

impl ParamAddress {
    /// 构造一个参数地址。
    #[must_use]
    pub const fn new(entity: EntityId, slot: u16) -> Self {
        Self { entity, slot }
    }
}

/// 走带（transport）命令。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportCommand {
    /// 开始播放。
    Play,
    /// 停止并回到起始位置。
    Stop,
    /// 暂停（保留当前位置）。
    Pause,
    /// 定位到指定 tick（960 PPQ，见 `yeban_model::PPQ`）。
    SeekTicks(u64),
}

/// 音频线程可消费的事件。
///
/// 必须是 `Copy`：批量 API（`push_partial_slice` / `pop_partial_slice`）要求 `T: Copy`，
/// 这也顺带保证"复制事件 = 复制字节"，没有隐藏的 `Drop`（**红线 7 禁止音频线程释放内存**）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EngineEvent {
    /// 空事件：临时缓冲的填充值，音频线程收到后不做任何事。
    Idle,
    /// 设置一个参数值（已归一化 / 已是目标域值，由模型层负责换域）。
    SetParam {
        /// 目标参数地址。
        target: ParamAddress,
        /// 目标值。
        value: f32,
    },
    /// 走带控制。
    Transport {
        /// 命令。
        command: TransportCommand,
    },
    /// 音符开（pitch 0..=127，velocity 0..=127）。
    NoteOn {
        /// 目标音轨。
        track: EntityId,
        /// MIDI 音高。
        pitch: u8,
        /// MIDI 力度。
        velocity: u8,
    },
    /// 音符关。
    NoteOff {
        /// 目标音轨。
        track: EntityId,
        /// MIDI 音高。
        pitch: u8,
    },
}

impl EngineEvent {
    /// 临时缓冲的推荐填充值（见 [`SCRATCH_EVENTS`]）。
    pub const IDLE: Self = Self::Idle;

    /// 该事件是否是空事件。
    #[must_use]
    pub fn is_idle(&self) -> bool {
        matches!(self, Self::Idle)
    }

    /// 该事件携带的参数目标（若适用）。
    #[must_use]
    pub fn param_target(&self) -> Option<ParamAddress> {
        match self {
            Self::SetParam { target, .. } => Some(*target),
            _ => None,
        }
    }
}

/// 事件通道的生产端（UI / 模型线程）。
///
/// 只应由**一个**线程持有（SPSC 的 "SP"）。`rtrb::Producer` 是 `Send` 但非 `Sync`，
/// 这一点由类型系统强制。
#[derive(Debug)]
pub struct EventSender {
    producer: Producer<EngineEvent>,
    bulk_push_calls: u64,
    events_pushed: u64,
}

impl EventSender {
    /// 通道容量（条）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.producer.buffer().capacity()
    }

    /// 还能写入多少条（不保证在读取后仍然成立：音频线程在并发消费）。
    #[must_use]
    pub fn free_slots(&self) -> usize {
        self.producer.slots()
    }

    /// 批量发布事件，返回**实际写入**的条数。
    ///
    /// 恰好调用一次 `Producer::push_partial_slice` [ARCH-RT-001, ROAD-M2-007]。
    /// 队列满时多出来的部分被丢弃（返回的条数 < `events.len()`），不阻塞、不分配。
    ///
    /// 生产端通常不是实时线程，但保持同一批量契约可以让"事件洪峰"不撕裂。
    pub fn publish(&mut self, events: &[EngineEvent]) -> usize {
        self.bulk_push_calls = self.bulk_push_calls.saturating_add(1);
        let (written, _remainder) = self.producer.push_partial_slice(events);
        let pushed = written.len();
        self.events_pushed = self.events_pushed.saturating_add(pushed as u64);
        pushed
    }

    /// 累计调用批量写 API 的次数（结构性判据用）。
    #[must_use]
    pub const fn bulk_push_calls(&self) -> u64 {
        self.bulk_push_calls
    }

    /// 累计成功写入的事件条数。
    #[must_use]
    pub const fn events_pushed(&self) -> u64 {
        self.events_pushed
    }
}

/// 事件通道的消费端（音频线程）。
///
/// 只应由**一个**线程持有（SPSC 的 "SC"）。
#[derive(Debug)]
pub struct EventReceiver {
    consumer: Consumer<EngineEvent>,
    bulk_pop_calls: u64,
    events_drained: u64,
}

impl EventReceiver {
    /// 通道容量（条）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.consumer.buffer().capacity()
    }

    /// 当前可读条数。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// 批量取出并逐个交给 `f`，返回取出的条数。
    ///
    /// **恰好调用一次** `Consumer::pop_partial_slice` [ARCH-RT-001, ROAD-M2-007]：
    /// 无论队列里有多少条，本函数只读一次原子索引、只翻转一次环形缓冲。
    ///
    /// `scratch` 必须是栈上定长数组（例如 `[EngineEvent::IDLE; 128]`）。
    /// 一次调用最多取出 `scratch.len()` 条；剩余的留到下一块（队列不会丢数据）。
    ///
    /// 闭包与 `scratch` 都不涉及堆分配；`f` 由单态化内联 [红线 7]。
    pub fn drain_with<F>(&mut self, scratch: &mut [EngineEvent], mut f: F) -> usize
    where
        F: FnMut(EngineEvent),
    {
        if scratch.is_empty() {
            return 0;
        }
        self.bulk_pop_calls = self.bulk_pop_calls.saturating_add(1);
        let (filled, _remainder) = self.consumer.pop_partial_slice(scratch);
        let drained = filled.len();
        for event in filled.iter().copied() {
            f(event);
        }
        self.events_drained = self.events_drained.saturating_add(drained as u64);
        drained
    }

    /// 批量取出到 `scratch` 前台，返回取出的条数（不做任何处理）。
    ///
    /// 与 [`drain_with`](Self::drain_with) 一样只调用一次批量 API。
    /// `scratch` 中下标 `>= 返回值` 的位置保持调用前的内容（即填充值）。
    pub fn drain_into(&mut self, scratch: &mut [EngineEvent]) -> usize {
        self.drain_with(scratch, |_| {})
    }

    /// 累计调用批量读 API 的次数（结构性判据用）。
    #[must_use]
    pub const fn bulk_pop_calls(&self) -> u64 {
        self.bulk_pop_calls
    }

    /// 累计取出的事件条数。
    #[must_use]
    pub const fn events_drained(&self) -> u64 {
        self.events_drained
    }
}

/// 建立一个事件通道。
///
/// `capacity` 就是**可容纳的条数**（实测 `rtrb 0.4` 的 `RingBuffer::capacity()` 原样返回
/// 传入值，不做 2 的幂取整）。容量为 0 的队列毫无意义，因此这里钳到至少 1。
#[must_use]
pub fn event_channel(capacity: usize) -> (EventSender, EventReceiver) {
    let (producer, consumer) = RingBuffer::<EngineEvent>::new(capacity.max(1));
    (
        EventSender {
            producer,
            bulk_push_calls: 0,
            events_pushed: 0,
        },
        EventReceiver {
            consumer,
            bulk_pop_calls: 0,
            events_drained: 0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn param(value: f32) -> EngineEvent {
        EngineEvent::SetParam {
            target: ParamAddress::new(EntityId::new(), 0),
            value,
        }
    }

    /// 判据 (d)：**每块只调用一次批量 API**（结构性断言，不依赖墙钟）。
    ///
    /// 构造一个必然需要多次搬运的场景（生产 3 批 × 40 条 = 120 条，临时缓冲只有 64 格），
    /// 断言 `bulk_pop_calls` 恰好等于 drain 次数：任何"逐条 pop"或"循环 pop 直到取空"
    /// 的实现都会让这个数字变大。
    #[test]
    fn bulk_contract_is_exactly_one_pop_call_per_block() {
        let (mut tx, mut rx) = event_channel(256);
        for block in 0..3u32 {
            let batch: Vec<EngineEvent> = (0..40).map(|i| param(block as f32 + i as f32)).collect();
            assert_eq!(tx.publish(&batch), 40);
        }
        assert_eq!(tx.bulk_push_calls(), 3, "生产端每批恰好一次批量写");
        assert_eq!(tx.events_pushed(), 120);

        let mut scratch = [EngineEvent::IDLE; 64];
        let mut drained = 0usize;
        for _ in 0..3 {
            drained += rx.drain_with(&mut scratch, |_| {});
        }
        assert_eq!(drained, 120, "第一次 64 + 第二次 56 + 第三次 0");
        assert_eq!(
            rx.bulk_pop_calls(),
            3,
            "3 次块处理必须恰好等于 3 次批量读 —— 这就是批量契约"
        );
        assert_eq!(rx.events_drained(), 120);
        assert_eq!(rx.pending(), 0);
    }

    /// 批量出队保持 FIFO 顺序，跨环形缓冲翻转点也不乱。
    #[test]
    fn drain_preserves_fifo_order_across_wraparound() {
        let (mut tx, mut rx) = event_channel(8);
        let mut expected: Vec<u32> = Vec::new();

        // 反复"写满 → 读空"，让环形缓冲在翻转点上工作。
        for round in 0..5u32 {
            let batch: Vec<EngineEvent> = (0..8)
                .map(|i| {
                    let serial = round * 8 + i;
                    expected.push(serial);
                    EngineEvent::NoteOn {
                        track: EntityId::new(),
                        pitch: serial as u8,
                        velocity: 100,
                    }
                })
                .collect();
            assert_eq!(tx.publish(&batch), 8, "round {round}: 应写满 8 条");

            let mut scratch = [EngineEvent::IDLE; 8];
            assert_eq!(rx.drain_into(&mut scratch), 8);
            for (slot, serial) in scratch.iter().zip(expected.drain(..)) {
                match slot {
                    EngineEvent::NoteOn {
                        pitch, velocity, ..
                    } => {
                        assert_eq!(u32::from(*pitch), serial);
                        assert_eq!(*velocity, 100);
                    }
                    other => panic!("顺序错乱: 期望 NoteOn, 实际 {other:?}"),
                }
            }
        }
        assert_eq!(rx.bulk_pop_calls(), 5);
    }

    /// 队列满时丢多余、不阻塞、不 panic；空队列读取返回 0。
    #[test]
    fn overflow_drops_excess_and_empty_drain_is_zero() {
        let (mut tx, mut rx) = event_channel(4);
        let batch: Vec<EngineEvent> = (0..10).map(|_| param(0.5)).collect();
        let accepted = tx.publish(&batch);
        assert_eq!(accepted, 4, "容量 4 的队列只能接受 4 条");
        assert_eq!(tx.events_pushed(), 4);

        let mut scratch = [EngineEvent::IDLE; 8];
        assert_eq!(rx.drain_into(&mut scratch), 4);
        assert_eq!(rx.pending(), 0);
        assert_eq!(
            rx.drain_into(&mut scratch),
            0,
            "空队列读取返回 0 而不是阻塞"
        );
        // 每次调用都仍然恰好一次批量 API（契约与空/满无关）
        assert_eq!(rx.bulk_pop_calls(), 2);
    }

    #[test]
    fn zero_length_scratch_does_not_call_the_bulk_api() {
        let (mut tx, mut rx) = event_channel(4);
        assert_eq!(tx.publish(&[param(1.0)]), 1);
        let mut empty: [EngineEvent; 0] = [];
        assert_eq!(rx.drain_into(&mut empty), 0);
        assert_eq!(rx.bulk_pop_calls(), 0, "空 scratch 不该浪费一次原子读取");
        assert_eq!(rx.pending(), 1, "事件仍在队列里");
    }

    #[test]
    fn events_are_copy_and_carry_no_destructor() {
        // 结构性判据: 事件是 Copy（无 Drop）⇒ 音频线程出队不会释放任何堆内存 [红线 7]。
        fn assert_copy<T: Copy>() {}
        assert_copy::<EngineEvent>();
        assert_copy::<ParamAddress>();
        assert!(
            size_of::<EngineEvent>() <= 64,
            "事件必须足够小, 能塞进批量缓冲"
        );
        assert!(EngineEvent::IDLE.is_idle());
        assert_eq!(EngineEvent::IDLE.param_target(), None);
        assert_eq!(param(1.0).param_target().map(|a| a.slot), Some(0));
        assert_eq!(
            EngineEvent::Transport {
                command: TransportCommand::SeekTicks(960)
            }
            .param_target(),
            None
        );
    }

    #[test]
    fn channel_capacity_is_at_least_one() {
        let (tx, rx) = event_channel(0);
        assert!(tx.capacity() >= 1);
        assert!(rx.capacity() >= 1);
        assert_eq!(tx.free_slots(), tx.capacity());
    }
}
