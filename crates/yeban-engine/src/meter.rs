//! VU / 峰值电平计量的独立 SPSC 解耦。[ARCH-UI-002, ROAD-M2-008]
//!
//! 规范原文（[ARCH-UI-002]）：
//!
//! > UI 线程绝不直接读取音频实时上下文。实时线程以 60Hz 速度向无锁 Meter SPSC
//! > 压入真峰值与 RMS 电平，UI 线程定时器批量出队更新 Slint Properties。
//!
//! 工程含义：
//!
//! 1. **独立队列**：电平数据量远大于参数事件（每轨每量子一条），必须与参数队列分开，
//!    否则电平洪峰会挤掉参数/音符事件 [ROAD-M2-008]；
//! 2. **高容量**：UI 以 60Hz 抽干，音频线程以 `sample_rate / block_frames` Hz 生产
//!    （48kHz / 128 = 375Hz）⇒ 一个 UI 帧积压 ~6 条/轨。默认容量 8192 条足够 64 轨；
//! 3. **有损**：队列满时**丢弃最新帧**并计数。电平是观测量，丢一帧只是 UI 少跳一次，
//!    而阻塞音频线程会直接爆音 —— 所以宁可丢。
//!
//! 生产侧同样使用批量 API（一次 `push_partial_slice` 推送本量子的全部轨），
//! 消费侧一次 `pop_partial_slice` 抽干。

use rtrb::{Consumer, Producer, RingBuffer};
use yeban_model::EntityId;

/// 电平队列默认容量（条）。
pub const DEFAULT_METER_CAPACITY: usize = 8192;

/// UI 侧建议的栈上临时缓冲长度。
pub const SCRATCH_METERS: usize = 256;

/// 一条电平计量帧（`Copy`，无 `Drop` ⇒ 音频线程出队不会释放内存 [红线 7]）。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MeterFrame {
    /// 计量对象（音轨 / 总线身份）。
    pub node: EntityId,
    /// 量子序号（`sample_rate / block_frames` 递增），UI 用它判断数据新鲜度。
    pub quantum: u64,
    /// 块内绝对值峰值（线性幅度，0.0..=1.0 为达标范围）。
    pub peak: f32,
    /// 块内 RMS（线性幅度）。
    pub rms: f32,
}

impl MeterFrame {
    /// 从一块样本算出一条计量帧（**不做任何分配**）。
    ///
    /// 空切片返回全零帧（RMS 定义为 0，而不是 `0/0 = NaN` —— NaN 会污染 UI 曲线）。
    #[must_use]
    pub fn measure(node: EntityId, quantum: u64, samples: &[f32]) -> Self {
        if samples.is_empty() {
            return Self {
                node,
                quantum,
                peak: 0.0,
                rms: 0.0,
            };
        }
        let mut peak = 0.0f32;
        let mut sum_squares = 0.0f64;
        for &sample in samples {
            let magnitude = sample.abs();
            if magnitude > peak {
                peak = magnitude;
            }
            sum_squares += f64::from(sample) * f64::from(sample);
        }
        let count = samples.len() as f64;
        Self {
            node,
            quantum,
            peak,
            rms: (sum_squares / count).sqrt() as f32,
        }
    }

    /// 峰值的 dBFS（`peak <= 0` 时返回负无穷）。
    #[must_use]
    pub fn peak_dbfs(&self) -> f32 {
        if self.peak <= 0.0 {
            f32::NEG_INFINITY
        } else {
            20.0 * self.peak.log10()
        }
    }
}

/// 电平队列的生产端（**音频线程**持有）。
#[derive(Debug)]
pub struct MeterPublisher {
    producer: Producer<MeterFrame>,
    bulk_push_calls: u64,
    events_pushed: u64,
    dropped: u64,
}

impl MeterPublisher {
    /// 队列容量（条）。
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.producer.buffer().capacity()
    }

    /// 批量推送本量子的全部电平帧，返回实际写入条数。
    ///
    /// 恰好一次 `push_partial_slice`（见 [`crate::ring`] 的批量契约）[ROAD-M2-007]。
    /// 队列满时多余帧被丢弃并计入 [`dropped`](Self::dropped) —— **绝不阻塞** [红线 7]。
    pub fn publish(&mut self, frames: &[MeterFrame]) -> usize {
        self.bulk_push_calls = self.bulk_push_calls.saturating_add(1);
        let (written, _remainder) = self.producer.push_partial_slice(frames);
        let pushed = written.len();
        self.events_pushed = self.events_pushed.saturating_add(pushed as u64);
        self.dropped = self.dropped.saturating_add((frames.len() - pushed) as u64);
        pushed
    }

    /// 累计批量写次数。
    #[must_use]
    pub const fn bulk_push_calls(&self) -> u64 {
        self.bulk_push_calls
    }

    /// 累计写入帧数。
    #[must_use]
    pub const fn frames_pushed(&self) -> u64 {
        self.events_pushed
    }

    /// 累计因队列满而丢弃的帧数（UI 若持续不抽，这个数字会上升 —— 可观测的健康指标）。
    #[must_use]
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }
}

/// 电平队列的消费端（**UI 线程**持有，60Hz 批量抽干）。
#[derive(Debug)]
pub struct MeterCollector {
    consumer: Consumer<MeterFrame>,
    bulk_pop_calls: u64,
    frames_drained: u64,
    ticks: u64,
}

impl MeterCollector {
    /// 当前积压帧数。
    #[must_use]
    pub fn pending(&self) -> usize {
        self.consumer.slots()
    }

    /// 一次 60Hz 抽取：把当前可读的帧批量搬到 `scratch` 前台，返回条数。
    ///
    /// 恰好一次 `pop_partial_slice`。一次抽不完就留到下一帧（下一帧再抽），
    /// 因此 `scratch` 给大一点可以显著降低"UI 永远追不上"的风险。
    pub fn tick(&mut self, scratch: &mut [MeterFrame]) -> usize {
        self.ticks = self.ticks.saturating_add(1);
        if scratch.is_empty() {
            return 0;
        }
        self.bulk_pop_calls = self.bulk_pop_calls.saturating_add(1);
        let (filled, _remainder) = self.consumer.pop_partial_slice(scratch);
        let drained = filled.len();
        self.frames_drained = self.frames_drained.saturating_add(drained as u64);
        drained
    }

    /// 累计批量读次数（结构性判据用）。
    #[must_use]
    pub const fn bulk_pop_calls(&self) -> u64 {
        self.bulk_pop_calls
    }

    /// 累计抽干帧数。
    #[must_use]
    pub const fn frames_drained(&self) -> u64 {
        self.frames_drained
    }

    /// 累计 60Hz tick 次数。
    #[must_use]
    pub const fn ticks(&self) -> u64 {
        self.ticks
    }
}

/// UI 侧"每节点最新电平"面板 [ARCH-UI-002]。
///
/// 电平是**观测量**：UI 只关心每节点最后一个值，中间的过渡帧可以直接覆盖。
/// 因此面板只需要"喂入本次抽到的帧 → 覆盖对应节点"，不需要保留历史。
///
/// 这个类型只活在 UI 线程上，所以允许 `BTreeMap` 插入（可能分配）。仍然用 `BTreeMap`
/// 而不是 `HashMap`，以保持与项目红线 4 一致的确定性迭代顺序（UI 元素顺序稳定）。
#[derive(Clone, Debug, Default)]
pub struct MeterBoard {
    latest: std::collections::BTreeMap<EntityId, MeterFrame>,
}

impl MeterBoard {
    /// 空面板。
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// 喂入一批帧；同节点的旧值被覆盖，返回被更新的节点数。
    pub fn ingest(&mut self, frames: &[MeterFrame]) -> usize {
        let mut updated = 0usize;
        for frame in frames {
            match self.latest.get(&frame.node) {
                Some(previous) if previous.quantum > frame.quantum => {}
                _ => {
                    self.latest.insert(frame.node, *frame);
                    updated += 1;
                }
            }
        }
        updated
    }

    /// 某个节点的最新电平。
    #[must_use]
    pub fn latest(&self, node: &EntityId) -> Option<&MeterFrame> {
        self.latest.get(node)
    }

    /// 已知节点数。
    #[must_use]
    pub fn len(&self) -> usize {
        self.latest.len()
    }

    /// 是否还没有任何电平。
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.latest.is_empty()
    }

    /// 按节点身份升序迭代（确定性）。
    pub fn iter(&self) -> impl Iterator<Item = (&EntityId, &MeterFrame)> {
        self.latest.iter()
    }
}

/// 建立电平队列：`Publisher` 给音频线程，`Collector` 给 UI 线程。
#[must_use]
pub fn meter_channel(capacity: usize) -> (MeterPublisher, MeterCollector) {
    let (producer, consumer) = RingBuffer::<MeterFrame>::new(capacity.max(1));
    (
        MeterPublisher {
            producer,
            bulk_push_calls: 0,
            events_pushed: 0,
            dropped: 0,
        },
        MeterCollector {
            consumer,
            bulk_pop_calls: 0,
            frames_drained: 0,
            ticks: 0,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measure_computes_peak_and_rms_without_nan_on_empty_input() {
        let node = EntityId::new();
        let frame = MeterFrame::measure(node, 3, &[1.0, -1.0, 0.0, 0.0]);
        assert_eq!(frame.peak, 1.0);
        // sqrt((1+1+0+0)/4) = sqrt(0.5)
        assert!((frame.rms - 0.5f32.sqrt()).abs() < 1e-6);
        assert_eq!(frame.quantum, 3);
        assert_eq!(frame.node, node);

        let empty = MeterFrame::measure(node, 0, &[]);
        assert_eq!(empty.rms, 0.0);
        assert!(!empty.rms.is_nan(), "空块不得产生 NaN");
        assert_eq!(empty.peak_dbfs(), f32::NEG_INFINITY);

        let full = MeterFrame::measure(node, 0, &[1.0, 1.0]);
        assert!(full.peak_dbfs().abs() < 1e-6, "满量程 = 0 dBFS");
    }

    /// 判据 (d)（电平侧）：音频线程**每量子一次**批量写、UI **每 tick 一次**批量读。
    #[test]
    fn meter_bulk_contract_is_one_call_per_quantum_and_per_ui_tick() {
        let (mut publisher, mut collector) = meter_channel(DEFAULT_METER_CAPACITY);
        let nodes: Vec<EntityId> = (0..4).map(|_| EntityId::new()).collect();
        let mut frames: Vec<MeterFrame> = Vec::new();

        // 模拟 10 个量子 × 4 轨
        for quantum in 0..10u64 {
            frames.clear();
            for node in &nodes {
                frames.push(MeterFrame {
                    node: *node,
                    quantum,
                    peak: 0.5,
                    rms: 0.25,
                });
            }
            assert_eq!(publisher.publish(&frames), 4);
        }
        assert_eq!(publisher.bulk_push_calls(), 10, "每个量子恰好一次批量写");
        assert_eq!(publisher.frames_pushed(), 40);
        assert_eq!(publisher.dropped(), 0);

        // UI 以 60Hz 抽干（这里两 tick 抽完）
        let mut scratch = [MeterFrame::default(); SCRATCH_METERS];
        let mut drained = 0;
        for _ in 0..2 {
            drained += collector.tick(&mut scratch);
        }
        assert_eq!(drained, 40);
        assert_eq!(collector.bulk_pop_calls(), 2, "每 tick 恰好一次批量读");
        assert_eq!(collector.ticks(), 2);
        assert_eq!(collector.pending(), 0);
    }

    #[test]
    fn meter_queue_is_lossy_but_never_blocks() {
        let (mut publisher, mut collector) = meter_channel(4);
        let node = EntityId::new();
        let frames: Vec<MeterFrame> = (0..10)
            .map(|quantum| MeterFrame {
                node,
                quantum,
                peak: 0.1,
                rms: 0.05,
            })
            .collect();
        assert_eq!(publisher.publish(&frames), 4, "容量 4 只能收 4 条");
        assert_eq!(publisher.dropped(), 6, "丢弃必须被计数, 便于观测健康度");

        let mut scratch = [MeterFrame::default(); 8];
        assert_eq!(collector.tick(&mut scratch), 4);
        assert_eq!(collector.tick(&mut scratch), 0, "空了就返回 0, 不阻塞");
    }

    #[test]
    fn board_keeps_only_the_latest_frame_per_node_and_ignores_stale_updates() {
        let a = EntityId::new();
        let b = EntityId::new();
        let mut board = MeterBoard::new();
        assert!(board.is_empty());

        let batch = [
            MeterFrame {
                node: a,
                quantum: 10,
                peak: 0.9,
                rms: 0.5,
            },
            MeterFrame {
                node: b,
                quantum: 10,
                peak: 0.2,
                rms: 0.1,
            },
        ];
        assert_eq!(board.ingest(&batch), 2);
        assert_eq!(board.len(), 2);

        // 同一个节点的新帧覆盖旧帧
        let newer = [MeterFrame {
            node: a,
            quantum: 11,
            peak: 0.1,
            rms: 0.02,
        }];
        assert_eq!(board.ingest(&newer), 1);
        assert_eq!(board.latest(&a).map(|f| f.quantum), Some(11));
        assert_eq!(board.latest(&a).map(|f| f.peak), Some(0.1));

        // 迟到的旧帧不得回退 UI（乱序/重放保护）
        let stale = [MeterFrame {
            node: a,
            quantum: 9,
            peak: 1.0,
            rms: 1.0,
        }];
        assert_eq!(board.ingest(&stale), 0, "旧量子不得覆盖新值");
        assert_eq!(board.latest(&a).map(|f| f.quantum), Some(11));

        // 迭代顺序按 EntityId（确定性）
        let order: Vec<EntityId> = board.iter().map(|(node, _)| *node).collect();
        let mut sorted = order.clone();
        sorted.sort_unstable();
        assert_eq!(order, sorted);
    }

    #[test]
    fn zero_scratch_tick_does_not_consume_the_queue() {
        let (mut publisher, mut collector) = meter_channel(8);
        let node = EntityId::new();
        assert_eq!(
            publisher.publish(&[MeterFrame {
                node,
                quantum: 0,
                peak: 1.0,
                rms: 1.0
            }]),
            1
        );
        let mut empty: [MeterFrame; 0] = [];
        assert_eq!(collector.tick(&mut empty), 0);
        assert_eq!(collector.bulk_pop_calls(), 0);
        assert_eq!(collector.pending(), 1, "零长 scratch 不该吞掉数据");
    }
}
