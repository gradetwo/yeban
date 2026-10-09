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
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
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

/// 夹具的**混音参数**（`line/engine-mix` 新增：声相与音色之前，夹具没有这两项）。
///
/// 默认值刻意取"与 `line/engine-sound` 的夹具完全一致"（音量 0 dB、声相居中、
/// 无设备链 ⇒ 音色旁通），这样既有的 9 条 `synth_render` 判据的**输入**没有被改动，
/// 只有输出因为声相定律/限制器而改变（见台账 §3 的口径变化表）。
#[derive(Clone, Copy, Debug)]
pub struct MixSpec {
    /// 音量 (dB)。
    pub volume_db: f32,
    /// 声相 -1.0..=1.0（0 = 居中）。
    pub pan: f32,
    /// 滤波器截止频率 (Hz)；`None` = 不挂乐器设备（音色旁通）。
    pub cutoff_hz: Option<f32>,
    /// 共振（0..1）。
    pub resonance: f32,
}

impl Default for MixSpec {
    fn default() -> Self {
        Self {
            volume_db: 0.0,
            pan: 0.0,
            cutoff_hz: None,
            resonance: 0.0,
        }
    }
}

impl MixSpec {
    /// 只改音量。
    #[must_use]
    pub const fn volume(volume_db: f32) -> Self {
        Self {
            volume_db,
            pan: 0.0,
            cutoff_hz: None,
            resonance: 0.0,
        }
    }

    /// 只改声相。
    #[must_use]
    pub const fn pan(pan: f32) -> Self {
        Self {
            volume_db: 0.0,
            pan,
            cutoff_hz: None,
            resonance: 0.0,
        }
    }

    /// 挂一个带截止频率的内置乐器设备（⇒ 声部滤波器启用）。
    #[must_use]
    pub const fn tone(cutoff_hz: f32, resonance: f32) -> Self {
        Self {
            volume_db: 0.0,
            pan: 0.0,
            cutoff_hz: Some(cutoff_hz),
            resonance,
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

/// 同 [`note_project`]，但轨道带上混音参数（音量 / 声相 / 内置乐器音色）。
///
/// 音色走的是**引擎侧临时形状**：`InternalInstrument` 设备的 `params` 里出现
/// `cutoff_hz` / `resonance` 即被 [`yeban_engine::synth::ToneParams::from_devices`] 采纳
/// （见该类型与台账 §5 的 needs）。模型的 `DeviceKind` 里没有"合成器"这一档，
/// 因此夹具用"内置乐器 + 约定参数名"表达，而不是去改模型。
#[must_use]
pub fn tuned_project(notes: &[NoteSpec], mix: MixSpec) -> Fixture {
    let mut fixture = note_project(notes);
    let track = fixture.track;
    let entry = fixture
        .project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.volume_db = mix.volume_db;
    entry.pan = mix.pan;
    if let Some(cutoff_hz) = mix.cutoff_hz {
        let device = EntityId::new();
        entry.devices = vec![DeviceDefinition {
            id: device,
            name: "Hollow".to_owned(),
            kind: DeviceKind::InternalInstrument,
            bypassed: false,
            params: vec![
                ParameterValue {
                    name: "cutoff_hz".to_owned(),
                    value: cutoff_hz,
                    unit: Some("Hz".to_owned()),
                },
                ParameterValue {
                    name: "resonance".to_owned(),
                    value: mix.resonance,
                    unit: None,
                },
            ],
            latency_samples: 0,
        }];
    }
    fixture
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

/// 主总线 + **两条** MIDI 轨（各自的片段与音符）+ 两条 `TrackToBus` 边。
///
/// `line/engine-wiring-2` 新增：插入链的判据需要**同一次渲染里两条轨挂不同器件**
/// （一条只有动态级、一条是完整通道条）⇒ 一次测量窗口就覆盖两条逐样本路径。
/// 与 [`note_project`] 同一个形状（同样的时间栅格、同样的产品路径），只是轨道数 = 2。
#[must_use]
pub fn two_track_project(
    first_notes: &[NoteSpec],
    second_notes: &[NoteSpec],
) -> (YebanProjectV1, EntityId, EntityId) {
    let master = EntityId::new();
    let first = EntityId::new();
    let second = EntityId::new();
    let mut tracks = BTreeMap::new();
    let mut pool = BTreeMap::new();
    let mut routing = RoutingGraph {
        nodes: vec![first, second, master],
        ..RoutingGraph::default()
    };

    for (track, notes) in [(first, first_notes), (second, second_notes)] {
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
        let mut midi_track = TrackV3 {
            id: track,
            name: "Track".to_owned(),
            kind: TrackKind::Midi,
            ..TrackV3::default()
        };
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
        tracks.insert(track, midi_track);

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
    }
    tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    (
        YebanProjectV1 {
            bpm: FIXTURE_BPM,
            master_bus_track_id: master,
            tracks,
            routing_graph: routing,
            clip_pool: pool,
            ..YebanProjectV1::default()
        },
        first,
        second,
    )
}

/// PDC 槽位重绑夹具里**长支路**的设备链上报延迟（采样点）。
///
/// 400 = 16 tick（1 tick = 25 样本）⇒ 落在夹具栅格上。它同时是两条短支路的补偿量
/// `D(v)`（`L_max = 400`），也就是"槽位换主人时环里留多少历史"的帧数。
pub const PDC_REBIND_LATENCY: u32 = 400;

/// PDC 槽位重绑夹具的**音符时值**（tick）：960 = 一个四分音符 = 24 000 帧。
pub const PDC_REBIND_NOTE_TICKS: u64 = 960;

/// PDC 槽位重绑夹具的**力度**。
///
/// 刻意压到母线限制器阈值之下：限制器一旦真的压限，它的增益弹道会带上远超
/// `BUS_LIMITER_LATENCY_FRAMES` 的记忆 ⇒"切换之后跳过那 33 帧就是逐位静音"
/// 这条窗口口径不再成立（不是缺陷，是夹具没设计好）。
const PDC_REBIND_VELOCITY: u8 = 40;

/// **PDC 槽位重绑**夹具的角色（`line/engine-20` 新增）。
///
/// 四个身份按 **ULID 升序**分配成固定角色。为什么必须排序：`CompensationBank::rearm`
/// 是**按 `PdcPlan::compensation` 的键序下标**逐槽绑定的（那是一个 `BTreeMap`，
/// 迭代顺序 = 身份升序）⇒"谁在哪个槽"完全由身份大小决定。判据要能预测"删掉一条轨
/// 之后谁挪进了谁的槽"，就只能自己把顺序钉住，否则测到的只是运气。
///
/// | 角色 | 键序 | 设备链上报延迟 | 音符 | 满计划 `D` | 剪枝后 `D` |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | [`removed`](Self::removed) | 1 | 0 | **有**（唯一出声的轨） | 400 | 不存在 |
/// | [`keeper`](Self::keeper) | 2 | 0 | 无 | 400 | 400 |
/// | [`slow`](Self::slow) | 3 | [`PDC_REBIND_LATENCY`] | 无 | 0 | 0 |
/// | [`master`](Self::master) | 4 | 0 | — | 0 | 0 |
///
/// 因此 `pruned` 一发布，`keeper` 就**下移进第 1 槽** —— 而那一槽的环里留着
/// `removed` 的 400 帧历史。
#[derive(Clone, Debug)]
pub struct PdcRebindFixture {
    /// 满计划：`removed` / `keeper` / `slow` / `master` 四条节点都在。
    pub project: YebanProjectV1,
    /// 剪掉 `removed` 之后的工程（同一组身份、同一条 `slow`、同一条 `master`）。
    pub pruned: YebanProjectV1,
    /// 被剪掉的支路（键序最小 ⇒ 占第 1 槽，且它有音频历史）。
    pub removed: EntityId,
    /// 幸存支路（键序第二 ⇒ 剪枝后下移进第 1 槽；它自己**没有音符**）。
    pub keeper: EntityId,
    /// 长支路（键序第三；它的上报延迟钉住 `L_max`，它自己**没有音符**）。
    pub slow: EntityId,
    /// 主总线（键序最大）。
    pub master: EntityId,
}

/// 构造 [`PdcRebindFixture`]。
///
/// ⚠ `[EntityId::new(); 4]` 只会调**一次** `new()` 再把那个值复制四份
/// （`EntityId` 是 `Copy`）⇒ 四个身份全相同、路由图会拼出自环。必须用
/// `core::array::from_fn`。
#[must_use]
pub fn pdc_rebind_fixture() -> PdcRebindFixture {
    let mut ids: [EntityId; 4] = core::array::from_fn(|_| EntityId::new());
    ids.sort_unstable();
    assert!(
        ids.windows(2).all(|pair| pair[0] != pair[1]),
        "四个身份必须两两不同：{ids:?}"
    );
    let [removed, keeper, slow, master] = ids;
    PdcRebindFixture {
        project: rebind_project(removed, keeper, slow, master, true),
        pruned: rebind_project(removed, keeper, slow, master, false),
        removed,
        keeper,
        slow,
        master,
    }
}

/// 一条没有任何片段的 MIDI 轨（渲染出来是逐位静音）。
fn rebind_silent_track(id: EntityId, name: &str) -> TrackV3 {
    TrackV3 {
        id,
        name: name.to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    }
}

/// 一条有一个 960 tick 长音符的 MIDI 轨（`t = 0` 起音，力度见
/// [`PDC_REBIND_VELOCITY`]）。
fn rebind_sounding_track(id: EntityId) -> (TrackV3, ClipPoolEntry) {
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note_id = EntityId::new();
    let mut note = MidiNote::new(note_id, 0, 60, PDC_REBIND_NOTE_TICKS);
    note.velocity = PDC_REBIND_VELOCITY;
    let entry = ClipPoolEntry {
        id: clip,
        name: "Rebind clip".to_owned(),
        content: ClipContent::Midi {
            notes: BTreeMap::from([(note_id, note)]),
        },
    };
    let mut track = rebind_silent_track(id, "Removed");
    track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: PDC_REBIND_NOTE_TICKS,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    (track, entry)
}

/// 组装夹具工程；`keep_removed = false` 时**整条** `removed` 轨（及其路由边）都不存在。
fn rebind_project(
    removed: EntityId,
    keeper: EntityId,
    slow: EntityId,
    master: EntityId,
    keep_removed: bool,
) -> YebanProjectV1 {
    let mut tracks = BTreeMap::new();
    let mut pool = BTreeMap::new();
    let mut nodes = vec![keeper, slow, master];

    if keep_removed {
        let (track, clip) = rebind_sounding_track(removed);
        pool.insert(clip.id, clip);
        tracks.insert(removed, track);
        nodes.push(removed);
    }
    tracks.insert(keeper, rebind_silent_track(keeper, "Keeper"));

    // `slow`：唯一带设备链上报延迟的轨。它不出声，只把 `L_max` 钉在
    // `PDC_REBIND_LATENCY` 上 ⇒ 两条短支路的 `D(v)` 恒等于它。
    let mut slow_track = rebind_silent_track(slow, "Slow");
    slow_track.devices = vec![DeviceDefinition {
        id: EntityId::new(),
        name: "Reported".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: Vec::new(),
        latency_samples: PDC_REBIND_LATENCY,
    }];
    tracks.insert(slow, slow_track);
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
        nodes,
        ..RoutingGraph::default()
    };
    for source in tracks.keys().copied().filter(|id| *id != master) {
        let edge = EntityId::new();
        routing.edges.insert(
            edge,
            RoutingEdge {
                id: edge,
                source_node: source,
                destination_node: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        );
    }

    YebanProjectV1 {
        bpm: FIXTURE_BPM,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
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
pub fn render_with<F>(project: &YebanProjectV1, quanta: usize, revision: u64, at: F) -> Render
where
    F: FnMut(usize, &mut Runtime),
{
    let snapshot =
        EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译成快照");
    render_snapshot_with(snapshot, quanta, at)
}

/// 同 [`render`]，但快照由调用方先构造好。
///
/// 用途：比较**同一条链的两条投影路径**（例如 `from_project` 与
/// `from_project_with_latencies`）产生的渲染是否逐位相同 —— 那需要两份不同的快照，
/// 而 [`render`] 只会自己调 `from_project`。
pub fn render_snapshot(snapshot: EngineSnapshot, quanta: usize) -> Render {
    render_snapshot_with(snapshot, quanta, |_, _| {})
}

/// [`render_with`] / [`render_snapshot`] 的共同实现（快照已就绪）。
fn render_snapshot_with<F>(snapshot: EngineSnapshot, quanta: usize, mut at: F) -> Render
where
    F: FnMut(usize, &mut Runtime),
{
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

// ---------------------------------------------------------------------------
// `line/engine-mix` 新增：声部级夹具（不经过模型 → 快照的投影）
// ---------------------------------------------------------------------------

/// 直接驱动 [`yeban_engine::synth::SynthEngine`] 的最小夹具。
///
/// 用途：声部窃取淡出、声部滤波器这些**声部级**行为在端到端路径上很难构造
/// （要撞复音上限、要挂设备链），而它们又是本线的核心判据。这里给出一个
/// **不经过模型投影**的入口：调度表由调用方手搓，音色由 `tones` 给。
///
/// 仍然走产品路径的**合成**部分：`begin_snapshot` → `render_track` →
/// 逐样本（整数相位 + ADSR + 可选滤波器）。只有"tick → 样本"这一步是手写的，
/// 因为那一部分的判据已经由 `synth_render.rs`（J3/J9）覆盖。
pub struct SynthRig {
    /// 被测合成器。
    pub engine: yeban_engine::synth::SynthEngine,
    /// 该轨身份。
    pub track: EntityId,
    /// 采样率。
    pub sample_rate: f32,
}

impl SynthRig {
    /// 组装：48 kHz、一条轨、给定音色。
    #[must_use]
    pub fn new(tones: &yeban_engine::synth::ToneParams) -> Self {
        let track = EntityId::new();
        let mut engine = yeban_engine::synth::SynthEngine::new(48_000);
        engine.begin_snapshot(48_000, &[track], [(&track, tones)], []);
        Self {
            engine,
            track,
            sample_rate: 48_000.0,
        }
    }

    /// 渲染一段（`quanta` 个 128 帧量子），返回单声道样本。
    #[must_use]
    pub fn render(
        &mut self,
        schedule: &yeban_engine::synth::NoteSchedule,
        quanta: usize,
    ) -> Vec<f32> {
        let mut out = vec![0.0f32; 128];
        let mut rendered = Vec::with_capacity(quanta * 128);
        for _ in 0..quanta {
            self.engine
                .render_track(self.track, Some(schedule), &mut out);
            self.engine.advance(128);
            rendered.extend_from_slice(&out);
        }
        rendered
    }
}

/// 造一个已调度音符（48 kHz、力度 127、`gain` 由调用方给）。
#[must_use]
pub fn scheduled(
    start_sample: u64,
    end_sample: u64,
    pitch: u8,
    gain: f32,
    sample_rate: f32,
) -> yeban_engine::synth::ScheduledNote {
    let freq = yeban_dsp_note_to_hz(pitch);
    yeban_engine::synth::ScheduledNote::new(
        start_sample,
        end_sample,
        pitch,
        127,
        freq,
        gain,
        sample_rate,
    )
}

/// 等程律频率（夹具用；与 `yeban_dsp::math::note_to_hz` 同式）。
#[must_use]
pub fn yeban_dsp_note_to_hz(pitch: u8) -> f32 {
    440.0 * 2.0f32.powf((f32::from(pitch) - 69.0) / 12.0)
}

/// 相邻样本的**最大**位移（爆音判据的核心读数）。
#[must_use]
pub fn max_step(samples: &[f32]) -> f32 {
    samples
        .windows(2)
        .fold(0.0f32, |worst, pair| worst.max((pair[1] - pair[0]).abs()))
}

/// 一次渲染里某个位置的**最大**相邻位移（用于"窃取发生处"的局部读数）。
#[must_use]
pub fn max_step_in(samples: &[f32], from: usize, to: usize) -> f32 {
    let to = to.min(samples.len());
    if from >= to {
        return 0.0;
    }
    max_step(&samples[from..to])
}

// ---------------------------------------------------------------------------
// `line/transport-engine` 新增：走带判据用的装配
// ---------------------------------------------------------------------------

/// 走带判据的装配：**保留事件生产端**，因此可以在量子之间真的发走带命令。
///
/// 与 [`Runtime`] 的区别只有一处：`Runtime` 刻意丢掉生产端（它不需要发命令），
/// 而走带判据必须能发 —— 走带命令走的是**产品路径**（`EngineEvent::Transport`
/// → 实时侧在量子边界出队应用），不是"直接调状态机"。两条路径都有判据：
/// 状态机本身在 `yeban_engine::transport` 的单元判据里，这里测的是**接线**。
pub struct TransportRig {
    /// 快照槽（控制线程侧）。
    pub slot: std::sync::Arc<SnapshotSlot>,
    /// 退役回收队列（主线程侧）。
    pub queue: yeban_engine::snapshot::RetireQueue,
    /// 事件生产端（控制线程侧）。
    pub sender: yeban_engine::ring::EventSender,
    /// 实时渲染驱动。
    pub runtime: EngineRuntime,
    /// 最近一个量子的交错输出（左/右）。
    pub output: Vec<f32>,
}

impl TransportRig {
    /// 按工程建装配（48 kHz 由 `audio_config` 决定；`quanta` 由调用方逐个驱动）。
    #[must_use]
    pub fn new(project: &YebanProjectV1, revision: u64) -> Self {
        let snapshot = EngineSnapshot::from_project(project, revision).expect("夹具工程必须能编译");
        let slot = SnapshotSlot::new(snapshot);
        let (retire, queue) = retire_channel(64);
        let (sender, receiver) = event_channel(64);
        let (publisher, _collector) = meter_channel(4096);
        let runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
        Self {
            slot,
            queue,
            sender,
            runtime,
            output: vec![0.0f32; 128 * 2],
        }
    }

    /// 发一批走带命令（**恰好一次**批量 API；返回实际写入条数）。
    pub fn send(&mut self, commands: &[yeban_engine::ring::TransportCommand]) -> usize {
        let events: Vec<yeban_engine::ring::EngineEvent> = commands
            .iter()
            .map(|command| yeban_engine::ring::EngineEvent::Transport { command: *command })
            .collect();
        self.sender.publish(&events)
    }

    /// 推一个 128 帧的量子（真实运行时量子长度 [ARCH-DET-001]）。
    pub fn quantum(&mut self) {
        self.output.fill(0.0);
        self.runtime.process_quantum(&mut self.output, 2);
    }

    /// 推 `count` 个量子。
    pub fn quanta(&mut self, count: usize) {
        for _ in 0..count {
            self.quantum();
        }
    }

    /// 左声道（本量子）。
    #[must_use]
    pub fn left(&self) -> Vec<f32> {
        self.output
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| pair[0])
            .collect()
    }

    /// 本量子两声道里非零样本数（"停住是否真的静音"的读数）。
    #[must_use]
    pub fn nonzero(&self) -> usize {
        self.output.iter().filter(|sample| **sample != 0.0).count()
    }
}
