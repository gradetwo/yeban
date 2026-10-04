//! 集成判据共用的**夹具与渲染驱动**（`line/engine-sound`）。
//!
//! 刻意自建工程而不是复用 `yeban_model::samples::filled_project()`：
//! `filled_project` 的编排是"给契约覆盖用的"（`probability`/`ratchet`/`micro_timing`
//! 各出现一次、`loop_config` 打开但区间等于摆放），它的**时间栅格不便于**判据
//! 精确断言"第几个样本起音"。本模块的夹具给出的是**可手算的栅格**：
//! 120 BPM / 48 kHz / 960 PPQ ⇒ **1 tick = 25 样本**。
//!
//! 所有夹具都通过 `EngineSnapshot::from_project` 走**产品路径**（不做任何测试专用
//! 捷径），因此判据测的是真实的模型 → 快照 → 实时链路。

#![allow(dead_code)]

use std::collections::BTreeMap;

use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::{EngineRuntime, EngineStats};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 判据用的采样率/速度：`1 tick = 60 × 48000 / (120 × 960) = 25` 样本。
pub const FIXTURE_BPM: f64 = 120.0;

/// 一个待摆放的音符（夹具输入）。
#[derive(Clone, Copy, Debug)]
pub struct NoteSpec {
    /// 起点 (tick)。
    pub start_tick: u64,
    /// 时值 (tick)。
    pub duration_ticks: u64,
    /// 音高。
    pub pitch: u8,
    /// 力度。
    pub velocity: u8,
}

impl NoteSpec {
    /// 中音 C、力度 100、四分音符（与 `MidiNote::default` 同口径）。
    #[must_use]
    pub const fn quarter(pitch: u8) -> Self {
        Self {
            start_tick: 0,
            duration_ticks: 960,
            pitch,
            velocity: 100,
        }
    }

    /// 指定起点/时值/音高/力度。
    #[must_use]
    pub const fn at(start_tick: u64, duration_ticks: u64, pitch: u8, velocity: u8) -> Self {
        Self {
            start_tick,
            duration_ticks,
            pitch,
            velocity,
        }
    }
}

/// 一个夹具工程的句柄。
#[derive(Clone, Debug)]
pub struct Fixture {
    /// 工程（可 clone 出任意多份**完全相同**的输入 ⇒ 确定性判据）。
    pub project: YebanProjectV1,
    /// 主总线节点。
    pub master: EntityId,
    /// 唯一的 MIDI 轨。
    pub track: EntityId,
}

/// 只有主总线、**没有任何轨道**的工程（"空工程"）。
#[must_use]
pub fn empty_project() -> YebanProjectV1 {
    let master = EntityId::new();
    YebanProjectV1 {
        master_bus_track_id: master,
        routing_graph: RoutingGraph {
            nodes: vec![master],
            ..RoutingGraph::default()
        },
        ..YebanProjectV1::default()
    }
}

/// 主总线 + 一条 MIDI 轨，**没有任何片段**的工程（"无音符工程"）。
#[must_use]
pub fn bare_track_project() -> Fixture {
    build(&[], TrackKind::Midi, true)
}

/// 主总线 + 一条 MIDI 轨 + 一个含给定音符的 MIDI 片段。
#[must_use]
pub fn note_project(notes: &[NoteSpec]) -> Fixture {
    build(notes, TrackKind::Midi, false)
}

/// 主总线 + 一条**音频**轨 + 一个 `ClipContent::Audio` 摆放（采样播放未接入 ⇒ 静音）。
#[must_use]
pub fn audio_clip_project() -> Fixture {
    let master = EntityId::new();
    let track = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();

    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Kick".to_owned(),
            content: ClipContent::Audio {
                asset: yeban_model::AssetHash::of_bytes(b"engine-sound-fixture"),
                gain_db: 0.0,
            },
        },
    );
    let mut placements = BTreeMap::new();
    placements.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 960,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );

    let mut tracks = BTreeMap::new();
    tracks.insert(
        track,
        TrackV3 {
            id: track,
            name: "Audio".to_owned(),
            kind: TrackKind::Audio,
            clips: placements,
            ..TrackV3::default()
        },
    );
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    Fixture {
        project: YebanProjectV1 {
            bpm: FIXTURE_BPM,
            master_bus_track_id: master,
            tracks,
            routing_graph: routing,
            clip_pool: pool,
            ..YebanProjectV1::default()
        },
        master,
        track,
    }
}

/// 通用构造：一条轨（给定类型）+ 可选的一个 MIDI 片段摆放。
#[must_use]
pub fn build(notes: &[NoteSpec], kind: TrackKind, without_clip: bool) -> Fixture {
    let master = EntityId::new();
    let track = EntityId::new();
    let mut tracks = BTreeMap::new();
    let mut pool = BTreeMap::new();

    let mut midi_track = TrackV3 {
        id: track,
        name: "Track".to_owned(),
        kind,
        ..TrackV3::default()
    };
    if !without_clip {
        let clip = EntityId::new();
        let placement = EntityId::new();
        let mut note_map = BTreeMap::new();
        for spec in notes {
            let id = EntityId::new();
            let mut note = MidiNote::new(id, spec.start_tick, spec.pitch, spec.duration_ticks);
            note.velocity = spec.velocity;
            note_map.insert(id, note);
        }
        pool.insert(
            clip,
            ClipPoolEntry {
                id: clip,
                name: "Clip".to_owned(),
                content: ClipContent::Midi { notes: note_map },
            },
        );
        let span = notes
            .iter()
            .map(|spec| spec.start_tick + spec.duration_ticks)
            .max()
            .unwrap_or(960);
        midi_track.clips.insert(
            placement,
            ClipPlacement {
                id: placement,
                clip_id: clip,
                start_tick: 0,
                duration_ticks: span,
                loop_config: LoopConfig::default(),
                muted: false,
            },
        );
    }
    tracks.insert(track, midi_track);
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: vec![track, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    routing.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    Fixture {
        project: YebanProjectV1 {
            bpm: FIXTURE_BPM,
            master_bus_track_id: master,
            tracks,
            routing_graph: routing,
            clip_pool: pool,
            ..YebanProjectV1::default()
        },
        master,
        track,
    }
}

/// 一次端到端渲染的结果。
#[derive(Clone, Debug)]
pub struct Render {
    /// 左声道（逐帧）。
    pub left: Vec<f32>,
    /// 右声道（逐帧）。
    pub right: Vec<f32>,
    /// 渲染结束时的引擎统计。
    pub stats: EngineStats,
    /// 渲染结束时音频线程持有的快照修订号。
    pub revision: Option<u64>,
}

impl Render {
    /// 帧数。
    #[must_use]
    pub fn frames(&self) -> usize {
        self.left.len()
    }

    /// 左右声道里**非零**的样本数。
    #[must_use]
    pub fn nonzero(&self) -> usize {
        self.left
            .iter()
            .chain(self.right.iter())
            .filter(|sample| **sample != 0.0)
            .count()
    }

    /// 峰值（绝对值最大）。
    #[must_use]
    pub fn peak(&self) -> f32 {
        self.left
            .iter()
            .chain(self.right.iter())
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    }

    /// 指纹：对**全部样本的位模式**做 FNV-1a 64（含左右两声道）。
    ///
    /// 用它做"逐位相同"的证据：打印一行十六进制就能在日志里人眼比对。
    #[must_use]
    pub fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for sample in self.left.iter().chain(self.right.iter()) {
            for byte in sample.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// 左声道样本的位模式（逐位比较用）。
    #[must_use]
    pub fn left_bits(&self) -> Vec<u32> {
        self.left.iter().map(|sample| sample.to_bits()).collect()
    }
}

/// 从工程渲染 `quanta` 个量子（每个量子 128 帧）。
///
/// 刻意走**产品路径**：`EngineSnapshot::from_project` → `SnapshotSlot` →
/// `EngineRuntime::process_quantum`。没有任何测试专用捷径。
#[must_use]
pub fn render(project: &YebanProjectV1, quanta: usize) -> Render {
    render_with(project, quanta, 1, |_, _| {})
}

/// 同 [`render`]，但允许在每个量子边界执行一个动作（用于"快照切换"判据）。
pub fn render_with<F>(project: &YebanProjectV1, quanta: usize, revision: u64, mut at: F) -> Render
where
    F: FnMut(usize, &mut Runtime),
{
    let snapshot =
        EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译成快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut rig = Runtime {
        slot,
        queue,
        runtime,
    };

    let mut left = Vec::with_capacity(quanta * 128);
    let mut right = Vec::with_capacity(quanta * 128);
    let mut out = vec![0.0f32; 128 * 2];
    for quantum in 0..quanta {
        at(quantum, &mut rig);
        out.fill(0.0);
        rig.runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
            right.push(out[frame * 2 + 1]);
        }
    }
    Render {
        left,
        right,
        stats: rig.runtime.stats(),
        revision: rig.runtime.revision(),
    }
}

/// 测试用的引擎装配（快照槽 + 退役队列 + 渲染驱动）。
pub struct Runtime {
    /// 快照槽（控制线程侧）。
    pub slot: std::sync::Arc<SnapshotSlot>,
    /// 退役回收队列（主线程侧）。
    pub queue: yeban_engine::snapshot::RetireQueue,
    /// 实时渲染驱动。
    pub runtime: EngineRuntime,
}

impl Runtime {
    /// 发布一份**等价**的新快照（只改 `revision`）⇒ 触发音频线程的快照切换。
    pub fn publish_equivalent(&self, project: &YebanProjectV1, revision: u64) {
        let snapshot = EngineSnapshot::from_project(project, revision).expect("等价快照");
        self.slot.publish(snapshot);
    }
}

/// 累计调谐（RMS）：返回 `(rms, peak)`。
#[must_use]
pub fn rms_peak(samples: &[f32]) -> (f64, f32) {
    let energy: f64 = samples
        .iter()
        .map(|sample| f64::from(*sample) * f64::from(*sample))
        .sum();
    let rms = if samples.is_empty() {
        0.0
    } else {
        (energy / samples.len() as f64).sqrt()
    };
    let peak = samples
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()));
    (rms, peak)
}

/// 零交叉计数（音高判据）。
#[must_use]
pub fn zero_crossings(samples: &[f32]) -> usize {
    samples
        .windows(2)
        .filter(|pair| (pair[0] < 0.0) != (pair[1] < 0.0))
        .count()
}
