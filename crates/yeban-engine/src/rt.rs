//! 渲染量子驱动：**不依赖 cpal** 的回调内逻辑。[ARCH-TOP-002, ARCH-RT-001]
//!
//! 本模块把"一个音频回调该做什么"完整实现成 [`EngineRuntime::process_quantum`]，
//! 并且**完全不引用 cpal**。这样做的理由是很具体的工程约束：
//!
//! - CI 上没有声卡，真实设备路径无法端到端运行；
//! - 但"回调逻辑"（事件批量出队 → 快照无锁切换 → 渲染 → 电平上报 → 退役入队）
//!   才是红线 7 的所在，必须可测。
//!
//! 于是分成两层：
//!
//! ```text
//!  EngineRuntime::process_quantum(&mut [f32], channels)   ← 本模块, 纯逻辑, 可单测
//!         ▲                                    ▲
//!         │ cpal 回调闭包                       │ device::NullBackend（无设备驱动）
//!  device::open_output(...)              device::NullBackend::render(frames)
//! ```
//!
//! ## 回调内禁令自检（[AGENTS.md §2 红线 7]）
//!
//! `process_quantum` 的调用树里**没有**：`Vec::push` / `Box::new` / `format!` / `println!`
//! / `Mutex::lock` / 文件或网络调用。所有临时缓冲都是结构体字段里的定长数组：
//!
//! - [`EngineRuntime::scratch_events`]：`[EngineEvent; 128]`（栈/内联）
//! - [`EngineRuntime::scratch_meters`]：`[MeterFrame; 256]`
//! - [`EngineRuntime::block`]：`AudioBlock<128>`（`[f32; 128]` × 2）
//!
//! 唯一允许的"共享状态"是原子量与 rtrb 队列；唯一的系统调用级别操作是
//! FTZ/DAZ 控制寄存器写入（一次）。

use std::sync::Arc;

use rtrb::Producer;

use crate::block::{AudioBlock, DEFAULT_BLOCK_FRAMES};
use crate::fpu::{self, FtzDazOutcome};
use crate::meter::{MeterFrame, MeterPublisher, SCRATCH_METERS};
use crate::ring::{EngineEvent, EventReceiver, SCRATCH_EVENTS};
use crate::snapshot::{EngineSnapshot, SnapshotReader, SnapshotSlot};

// 编译期钉住块长是规范允许的取值 [ARCH-DET-001, MODEL-AST-002]。
const _: () = crate::block::assert_supported_frames::<DEFAULT_BLOCK_FRAMES>();
// 栈上临时事件缓冲至少能装下一个量子的参数洪峰。
const _: () = assert!(SCRATCH_EVENTS >= DEFAULT_BLOCK_FRAMES);

/// 渲染驱动的累计统计（音频线程写入，非实时线程读取 —— 只用于诊断/UI）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EngineStats {
    /// 累计渲染量子数。
    pub quanta: u64,
    /// 累计应用的事件条数（不含 `Idle` 填充）。
    pub events_applied: u64,
    /// 累计快照切换次数。
    pub snapshot_switches: u64,
    /// 累计批量出队调用次数（结构性判据：应等于 `quanta`）。
    pub event_bulk_pops: u64,
    /// 累计上报的电平帧数。
    pub meter_frames: u64,
    /// 本线程的 FTZ/DAZ 开关结果。
    pub ftz: Option<FtzDazOutcome>,
}

/// 渲染驱动：音频回调持有的全部可变状态。
pub struct EngineRuntime {
    snapshot: SnapshotReader,
    events: EventReceiver,
    meters: MeterPublisher,
    block: AudioBlock<DEFAULT_BLOCK_FRAMES>,
    scratch_events: [EngineEvent; SCRATCH_EVENTS],
    scratch_meters: [MeterFrame; SCRATCH_METERS],
    quanta: u64,
    events_applied: u64,
    event_bulk_pops: u64,
    meter_frames: u64,
    ftz: Option<FtzDazOutcome>,
    ftz_ready: bool,
}

impl EngineRuntime {
    /// 组装渲染驱动。
    ///
    /// `slot` / `retire` 来自 [`SnapshotSlot`] 与 [`crate::snapshot::retire_channel`]；
    /// `events` / `meters` 来自 [`crate::ring::event_channel`] 与 [`crate::meter::meter_channel`]。
    /// 全部通道都必须在**打开设备之前**建立（回调内不允许分配）。
    #[must_use]
    pub fn new(
        slot: &Arc<SnapshotSlot>,
        retire: Producer<Arc<EngineSnapshot>>,
        events: EventReceiver,
        meters: MeterPublisher,
    ) -> Self {
        Self {
            snapshot: SnapshotReader::attach(slot, retire),
            events,
            meters,
            block: AudioBlock::new(),
            scratch_events: [EngineEvent::IDLE; SCRATCH_EVENTS],
            scratch_meters: [MeterFrame::default(); SCRATCH_METERS],
            quanta: 0,
            events_applied: 0,
            event_bulk_pops: 0,
            meter_frames: 0,
            ftz: None,
            ftz_ready: false,
        }
    }

    /// 处理一个（可能是任意长度的）输出缓冲：按 [`DEFAULT_BLOCK_FRAMES`] 切成整量子。
    ///
    /// `output` 是 cpal 给出的**交错**缓冲；`channels` 是通道数。长度不是
    /// `channels * k` 时，尾部不足一帧的样本保持原值（cpal 保证输出缓冲预填静音）。
    ///
    /// 本函数是实时路径：零分配、零锁、零阻塞 I/O [红线 7]。
    pub fn process_quantum(&mut self, output: &mut [f32], channels: u16) {
        self.arm_fpu_once();
        let channels = usize::from(channels.max(1));
        let total_frames = output.len() / channels;
        let mut offset = 0usize;
        while offset < total_frames {
            let frames = (total_frames - offset).min(DEFAULT_BLOCK_FRAMES);
            self.render_block(frames);
            for channel in 0..channels {
                for frame in 0..frames {
                    let sample = self.block.get(channel, frame).unwrap_or(0.0);
                    output[(offset + frame) * channels + channel] = sample;
                }
            }
            offset += frames;
        }
        // 帧边界对齐的交错缓冲不会留下尾巴；非对齐的残余保持 cpal 预填的静音。
    }

    /// 当前累计统计。
    #[must_use]
    pub fn stats(&self) -> EngineStats {
        EngineStats {
            quanta: self.quanta,
            events_applied: self.events_applied,
            snapshot_switches: self.snapshot.switches(),
            event_bulk_pops: self.event_bulk_pops,
            meter_frames: self.meter_frames,
            ftz: self.ftz,
        }
    }

    /// 当前快照的模型层版本号（音频线程是否已追上模型线程）。
    #[must_use]
    pub fn revision(&self) -> Option<u64> {
        self.snapshot.held().map(|snapshot| snapshot.revision())
    }

    /// 最近一个量子的输出块（测试与诊断用；只读）。
    #[must_use]
    pub fn last_block(&self) -> &AudioBlock<DEFAULT_BLOCK_FRAMES> {
        &self.block
    }

    /// FTZ/DAZ 是否已在本线程生效 [ARCH-RT-003]。
    #[must_use]
    pub fn ftz_armed(&self) -> bool {
        self.ftz_ready
    }

    /// 每个回调线程**一次**：开启 FTZ/DAZ。
    ///
    /// 用普通 `bool` 标志而不是 `Once`：`Once` 在竞争路径上会**阻塞等待**，
    /// 那正是红线 7 禁止的锁等待。本标志只被音频线程读写，因此不需要原子量、
    /// 也不需要同步。控制寄存器是线程局部的，所以必须在这里（回调线程内）设置，
    /// 而不是在打开设备的主线程上 [ARCH-RT-003]。
    fn arm_fpu_once(&mut self) {
        if !self.ftz_ready {
            self.ftz = Some(fpu::enable_ftz_daz());
            self.ftz_ready = true;
        }
    }

    /// 渲染一个量子。
    fn render_block(&mut self, frames: usize) {
        let Self {
            snapshot,
            events,
            meters,
            block,
            scratch_events,
            scratch_meters,
            quanta,
            events_applied,
            event_bulk_pops,
            meter_frames,
            ..
        } = self;

        *quanta = quanta.wrapping_add(1);

        // --- 1) 参数/音符/走带事件：**每块一次**批量出队 [ROAD-M2-007] ---
        let mut applied = 0usize;
        events.drain_with(scratch_events, |event| {
            if !event.is_idle() {
                applied += 1;
            }
        });
        *event_bulk_pops = event_bulk_pops.wrapping_add(1);
        *events_applied = events_applied.wrapping_add(applied as u64);

        // --- 2) 快照边界处的无锁切换 [ARCH-RT-002] ---
        let quantum = *quanta;
        let mut rendered_tracks = 0usize;
        if let Some(current) = snapshot.begin_block() {
            // 渲染占位：真正的声部合成/通道条在后续切片接入（见模块文档的边界说明）。
            // 这里先把"块长来自快照"和"输出块被清空"两条契约落实，避免下游拿到陈旧样本。
            block.silence();
            block.set_frames(frames);

            // --- 3) 电平：每轨一条 + master 一条，一次批量推送 [ROAD-M2-008] ---
            for id in current.tracks().keys() {
                if rendered_tracks >= scratch_meters.len() {
                    break;
                }
                scratch_meters[rendered_tracks] = MeterFrame::measure(*id, quantum, block.left());
                rendered_tracks += 1;
            }
            if rendered_tracks < scratch_meters.len() {
                scratch_meters[rendered_tracks] =
                    MeterFrame::measure(current.master(), quantum, block.left());
                rendered_tracks += 1;
            }
        } else {
            // 极端情况（写者尚未发布任何快照）：输出静音但绝不 panic。
            block.silence();
            block.set_frames(frames);
        }
        snapshot.end_block();

        if rendered_tracks > 0 {
            let published = meters.publish(&scratch_meters[..rendered_tracks]);
            *meter_frames = meter_frames.wrapping_add(published as u64);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::LatencyTable;
    use crate::meter::meter_channel;
    use crate::ring::event_channel;
    use crate::snapshot::retire_channel;
    use std::collections::BTreeMap;
    use yeban_model::{EntityId, RoutingEdge, RoutingGraph, RoutingKind};

    fn simple_snapshot(revision: u64) -> EngineSnapshot {
        let master = EntityId::new();
        let track = EntityId::new();
        let mut routing = RoutingGraph {
            nodes: vec![track, master],
            ..RoutingGraph::default()
        };
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: track,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
        let mut tracks = BTreeMap::new();
        let track_model = yeban_model::TrackV3 {
            id: track,
            ..yeban_model::TrackV3::default()
        };
        tracks.insert(
            track,
            crate::snapshot::TrackParams::from_track(&track_model, 0),
        );
        EngineSnapshot::from_parts(
            revision,
            48_000,
            DEFAULT_BLOCK_FRAMES,
            2,
            master,
            tracks,
            &routing,
            &LatencyTable::new(),
        )
        .expect("合法图")
    }

    struct Rig {
        slot: Arc<SnapshotSlot>,
        queue: crate::snapshot::RetireQueue,
        sender: crate::ring::EventSender,
        collector: crate::meter::MeterCollector,
        runtime: EngineRuntime,
    }

    fn rig() -> Rig {
        let slot = SnapshotSlot::new(simple_snapshot(1));
        let (retire, queue) = retire_channel(16);
        let (sender, receiver) = event_channel(64);
        let (publisher, collector) = meter_channel(256);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Rig {
            slot,
            queue,
            sender,
            collector,
            runtime,
        }
    }

    /// 编译期判据：渲染驱动必须能 move 进 cpal 的回调闭包（`D: Send + 'static`）。
    ///
    /// `NullBackend` 属于 `device` feature（它的配置类型含 cpal 类型），
    /// 因此那条断言按 feature 门控 [ADR-0001 D19]。
    #[test]
    fn runtime_is_send_so_it_can_move_into_the_audio_callback() {
        fn assert_send<T: Send>() {}
        assert_send::<EngineRuntime>();
        #[cfg(feature = "device")]
        assert_send::<crate::device::NullBackend>();
    }

    #[test]
    fn null_path_renders_silence_and_counts_quanta() {
        let mut rig = rig();
        let mut out = [1.5f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        assert!(out.iter().all(|s| *s == 0.0), "占位渲染必须输出静音");
        let stats = rig.runtime.stats();
        assert_eq!(stats.quanta, 1);
        assert_eq!(stats.event_bulk_pops, 1, "每量子一次批量出队");
        assert_eq!(stats.meter_frames, 2, "一条音轨 + 一条 master");
        assert!(rig.runtime.ftz_armed());
        assert_eq!(rig.runtime.revision(), Some(1));
    }

    #[test]
    fn partial_and_multi_quantum_buffers_are_split_at_the_boundary() {
        let mut rig = rig();
        // 非整块: 200 帧 = 128 + 72
        let mut out = [0.5f32; 400];
        rig.runtime.process_quantum(&mut out, 2);
        assert_eq!(rig.runtime.stats().quanta, 2, "200 帧必须切成两个量子");
        assert_eq!(rig.runtime.stats().event_bulk_pops, 2);
        assert!(out.iter().all(|s| *s == 0.0));
        assert_eq!(rig.runtime.last_block().frames(), 72);
    }

    #[test]
    fn events_are_drained_once_per_block_and_applied() {
        let mut rig = rig();
        let target = crate::ring::ParamAddress::new(EntityId::new(), 3);
        let batch: Vec<EngineEvent> = (0..5)
            .map(|i| EngineEvent::SetParam {
                target,
                value: i as f32,
            })
            .collect();
        assert_eq!(rig.sender.publish(&batch), 5);
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);
        let stats = rig.runtime.stats();
        assert_eq!(stats.events_applied, 5);
        assert_eq!(stats.event_bulk_pops, 1, "5 个事件仍然只调用一次批量 API");
    }

    #[test]
    fn snapshot_swap_moves_old_arc_into_retire_queue_and_never_drops_in_rt() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        rig.runtime.process_quantum(&mut out, 2);

        rig.slot.publish(simple_snapshot(2));
        rig.slot.publish(simple_snapshot(3));
        rig.runtime.process_quantum(&mut out, 2);

        assert_eq!(rig.runtime.revision(), Some(3), "音频线程追上了最新快照");
        assert_eq!(
            rig.runtime.stats().snapshot_switches,
            1,
            "两次 publish 只切一次"
        );
        assert_eq!(rig.queue.pending(), 1, "旧快照被 move 进退役队列");
        assert_eq!(rig.queue.drain(4), 1, "主线程负责 Drop");
        assert!(rig.slot.prune() >= 1, "读者推进后写者侧清单也可回收");
    }

    #[test]
    fn meter_frames_are_published_per_track_with_bulk_api() {
        let mut rig = rig();
        let mut out = [0.0f32; DEFAULT_BLOCK_FRAMES * 2];
        for _ in 0..5 {
            rig.runtime.process_quantum(&mut out, 2);
        }
        let mut scratch = [MeterFrame::default(); 64];
        let drained = rig.collector.tick(&mut scratch);
        assert_eq!(drained, 10, "5 个量子 × 2 条计量");
        assert_eq!(rig.collector.bulk_pop_calls(), 1, "UI 一次 tick 一次批量读");
        assert!(scratch[..drained].iter().all(|f| f.peak == 0.0));
        // 量子序号必须递增（UI 用它做新鲜度判断）
        assert!(scratch[0].quantum < scratch[2].quantum);
    }

    #[test]
    fn odd_channel_counts_and_empty_buffers_do_not_panic() {
        let mut rig = rig();
        let mut out = [0.7f32; 5]; // 5 个样本 / 2 声道 = 2 帧 + 1 个残余样本
        rig.runtime.process_quantum(&mut out, 2);
        assert_eq!(rig.runtime.stats().quanta, 1);
        // 残余样本保持原值（cpal 会预填静音, 这里只断言没有越界写）
        assert_eq!(out[4], 0.7);
        let mut empty: [f32; 0] = [];
        rig.runtime.process_quantum(&mut empty, 2);
        assert_eq!(rig.runtime.stats().quanta, 1, "空缓冲不产生量子");
    }
}
