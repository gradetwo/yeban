//! 任务单 `#158`：**总线（aux 返回）上的插入链** —— 让 `SendToAux` 送进 Aux 的
//! 信号在**实时监听**里也过那条总线自己的 EQ / 混响 / 卷积混响。
//!
//! `#154` 把路由图接进了实时引擎，但总线暂存池里的**累积信号**只乘边增益 ⇒ 送到 aux
//! 的只是"干"总线：混响只有**导出**里响（离线 `RenderPlan` 在**求和之后**给每个
//! 非 master 节点跑 `InsertChain`），监听里听不到。本文件判的就是这条缺口被补上。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | B1 | 总线 `reverb_wet: 1` ⇒ 音符结束后 **200–700 ms** 的尾窗 RMS 显著高于 `reverb_wet: 0`；且 `insert_reverb_frames == 渲染帧数`（每量子每节点**恰好一次**） | 转发前不跑链（尾窗 = 0，计数 0）/ 逐轨与转发各跑一次（计数 2×、尾巴两倍速衰减） |
//! | B2 | 总线 `eq_low_gain: +12` ⇒ 低频夹具 RMS 上升（给 dB 读数） | 总线 EQ 没接 / 参数没进器件 |
//! | B3 | `bypassed = true` ⇒ 回到干信号（与"没有设备"逐位相同）且三个插入计数全 0 | 旁通设备仍被武装 |
//! | B4 | **快路**：无设备总线 ⇒ 三个插入计数 0、输出与**直连 master** 逐位相同；同夹具挂设备 ⇒ 计数 > 0 且输出不同 | 无设备总线也进插入路径（下混会改变**硬声相**的 L/R） |
//! | B5 | 尺寸红线 ＋ 三件器件类型的 `size_of` 读数（裁决 (甲) 的代价背景） | 为"左右各一套"加每总线状态 |
//! | B6 | **卷积混响**也在总线上生效：`conv_wet: 1` 与无设备臂的**差信号**（＝纯湿输出）在 IR 长度窗内非零；`insert_convolution_frames == 渲染帧数` | 卷积级没接 / 只跑在逐轨路径 |
//!
//! 判据全部走**产品路径**：`YebanProjectV1` → `EngineSnapshot::from_project` →
//! `EngineRuntime::process_quantum`（`support::render`）。夹具的时间栅格与
//! `tests/bus_routing.rs` 相同：120 BPM / 48 kHz / 960 PPQ ⇒ **1 tick = 25 样本**。
//!
//! ⚠ **本文件不证明"与导出逐位相同"**：实时总线插入是**下混成单声道**（裁决 (甲)），
//! 离线是**真立体声**插入；对齐的是"有/没有"插入，不是逐位（报告 §7 登记）。

mod support;

use std::collections::BTreeMap;

use support::{NoteSpec, render, rms_peak};
use yeban_engine::insert::{ChannelStrip, ConvolutionReverb, Reverb};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::sfz_instrument::{PreparedRegion, SfzVoice, SfzVoicePool};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, DeviceDefinition, DeviceKind, EntityId, LoopConfig,
    MidiNote, ParameterValue, RoutingEdge, RoutingGraph, RoutingKind, TrackKind, TrackV3,
    YebanProjectV1,
};

/// B1 的渲染长度：2,400 × 128 = 307,200 帧 = 6.4 s @48 kHz（尾窗必须落在音符之后很远）。
const TAIL_QUANTA: usize = 2_400;

/// B2/B3/B4 的渲染长度：400 × 128 = 51,200 帧 ≈ 1.07 s（足够覆盖 1 s 的音符）。
const SHORT_QUANTA: usize = 400;

/// 一个待摆放的音符（与 `support::NoteSpec` 同形，这里只做别名，便于阅读）。
type Note = NoteSpec;

/// 一条测试轨的声明。
struct TrackSpec {
    id: EntityId,
    kind: TrackKind,
    pan: f32,
    volume_db: f32,
    notes: Vec<Note>,
    devices: Vec<DeviceDefinition>,
}

impl TrackSpec {
    fn midi_notes(id: EntityId, pan: f32, volume_db: f32, notes: Vec<Note>) -> Self {
        Self {
            id,
            kind: TrackKind::Midi,
            pan,
            volume_db,
            notes,
            devices: Vec::new(),
        }
    }

    fn aux(id: EntityId, devices: Vec<DeviceDefinition>) -> Self {
        Self {
            id,
            kind: TrackKind::AuxReturn,
            pan: 0.0,
            volume_db: 0.0,
            notes: Vec::new(),
            devices,
        }
    }
}

/// 一条测试边。
struct EdgeSpec {
    source: EntityId,
    destination: EntityId,
    kind: RoutingKind,
}

/// 一张效果器设备（`InternalEffect`，与 `tests/reverb_insert.rs` 同款）。
fn effect(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Effect".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
    }
}

/// 一台混响设备；`wet = 0` 是"武装但器件的 `is_active()` 为假"的那一档。
fn reverb_device(wet: f32, size: f32) -> DeviceDefinition {
    effect(&[("reverb_size", size), ("reverb_wet", wet)])
}

/// 组装工程（音轨 + 片段池 + 路由图）。
fn build_project(master: EntityId, tracks: &[TrackSpec], edges: &[EdgeSpec]) -> YebanProjectV1 {
    let mut map = BTreeMap::new();
    let mut pool = BTreeMap::new();
    for spec in tracks {
        let mut track = TrackV3 {
            id: spec.id,
            name: "Track".to_owned(),
            kind: spec.kind,
            pan: spec.pan,
            volume_db: spec.volume_db,
            devices: spec.devices.clone(),
            ..TrackV3::default()
        };
        if !spec.notes.is_empty() {
            let clip = EntityId::new();
            let placement = EntityId::new();
            let mut note_map = BTreeMap::new();
            for note in &spec.notes {
                let id = EntityId::new();
                let mut entry = MidiNote::new(id, note.start_tick, note.pitch, note.duration_ticks);
                entry.velocity = note.velocity;
                note_map.insert(id, entry);
            }
            pool.insert(
                clip,
                ClipPoolEntry {
                    id: clip,
                    name: "Clip".to_owned(),
                    content: ClipContent::Midi { notes: note_map },
                },
            );
            let span = spec
                .notes
                .iter()
                .map(|note| note.start_tick + note.duration_ticks)
                .max()
                .unwrap_or(480);
            track.clips.insert(
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
        map.insert(spec.id, track);
    }
    map.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );

    let mut routing = RoutingGraph {
        nodes: Vec::new(),
        ..RoutingGraph::default()
    };
    for spec in tracks {
        routing.nodes.push(spec.id);
    }
    routing.nodes.push(master);
    for edge in edges {
        let id = EntityId::new();
        routing.edges.insert(
            id,
            RoutingEdge {
                id,
                source_node: edge.source,
                destination_node: edge.destination,
                kind: edge.kind,
                gain_db: None,
            },
        );
    }

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks: map,
        routing_graph: routing,
        clip_pool: pool,
        ..YebanProjectV1::default()
    }
}

/// 夹具：轨 A `SendToAux` → aux → `BusToMaster` → master。
fn send_project(
    note: Note,
    pan: f32,
    volume_db: f32,
    aux_devices: Vec<DeviceDefinition>,
) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    let aux = EntityId::new();
    build_project(
        master,
        &[
            TrackSpec::midi_notes(track, pan, volume_db, vec![note]),
            TrackSpec::aux(aux, aux_devices),
        ],
        &[
            EdgeSpec {
                source: track,
                destination: aux,
                kind: RoutingKind::SendToAux,
            },
            EdgeSpec {
                source: aux,
                destination: master,
                kind: RoutingKind::BusToMaster,
            },
        ],
    )
}

/// 对照夹具：轨 A **直连 master**（没有 aux，也没有路由边）。
fn direct_project(note: Note, pan: f32) -> YebanProjectV1 {
    let master = EntityId::new();
    let track = EntityId::new();
    build_project(
        master,
        &[TrackSpec::midi_notes(track, pan, 0.0, vec![note])],
        &[],
    )
}

/// 尾窗（B1）：音符在 tick 960 结束 = 样本 **24,000**；200 ms = 9,600 样本，
/// 700 ms = 33,600 样本 ⇒ `[33_600, 57_600)`。2,400 个量子 = 307,200 帧 ⇒ 窗口在内。
const TAIL_WINDOW: core::ops::Range<usize> = 33_600..57_600;

/// 线性 RMS → dBFS（与 `tests/channel_strip_insert.rs` 的 C10 同一把尺子）。
fn dbfs(linear: f64) -> f64 {
    20.0 * (linear.max(1e-12)).log10()
}

/// 快照里这份工程**确实**启用了总线暂存池（否则本文件的夹具根本没走到那条路径）。
fn assert_uses_buses(project: &YebanProjectV1) {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具必须能编译成快照");
    assert!(
        snapshot.routing().uses_buses(),
        "夹具必须有非 master 目的地（否则总线转发段不进）"
    );
}

/// B1：**实时总线上有混响**。
///
/// 判据：音符结束后 200–700 ms 的尾窗里，`reverb_wet: 1` 的 RMS 显著高于
/// `reverb_wet: 0`；并且 `insert_reverb_frames` **恰好等于渲染帧数**（每量子每节点
/// 一次 —— 若逐轨阶段也跑，计数会翻倍、混响尾巴会以两倍速衰减）。
#[test]
fn b1_the_bus_reverb_tail_is_audible_after_the_note_ends() {
    let note = Note::at(0, 960, 48, 40);
    let dry_project = send_project(note, 0.0, 0.0, vec![reverb_device(0.0, 0.8)]);
    let wet_project = send_project(note, 0.0, 0.0, vec![reverb_device(1.0, 0.8)]);
    assert_uses_buses(&wet_project);
    let dry = render(&dry_project, TAIL_QUANTA);
    let wet = render(&wet_project, TAIL_QUANTA);

    let (dry_tail, _) = rms_peak(&dry.left[TAIL_WINDOW]);
    let (wet_tail, _) = rms_peak(&wet.left[TAIL_WINDOW]);
    let expected_frames = (TAIL_QUANTA * 128) as u64;
    println!(
        "[#158/B1] 尾窗 [{}..{})：wet=0 RMS={:.9}（{:.3} dBFS）；wet=1 RMS={:.9}（{:.3} dBFS）；\
         两档差={:+.3} dB；insert_reverb_frames: wet=0 {} / wet=1 {}（渲染帧数 {}）；\
         limiter_gain_reductions={}",
        TAIL_WINDOW.start,
        TAIL_WINDOW.end,
        dry_tail,
        dbfs(dry_tail),
        wet_tail,
        dbfs(wet_tail),
        dbfs(wet_tail) - dbfs(dry_tail),
        dry.stats.insert_reverb_frames,
        wet.stats.insert_reverb_frames,
        expected_frames,
        wet.stats.limiter_gain_reductions,
    );

    assert_eq!(
        dry.stats.insert_reverb_frames, 0,
        "wet=0 ⇒ 器件的 is_active() 为假 ⇒ 状态不推进、计数必须为 0"
    );
    assert_eq!(
        wet.stats.insert_reverb_frames, expected_frames,
        "总线混响必须**恰好每量子处理一次**（每节点每量子一次）"
    );
    assert!(
        dry_tail < 1e-5,
        "wet=0 的尾窗必须是静音（release 0.05 s ≪ 200 ms）：{dry_tail:.9}"
    );
    assert!(
        wet_tail > 1e-3,
        "wet=1 的尾窗必须听到混响尾巴（> 1e-3）：{wet_tail:.9}"
    );
    assert!(
        wet_tail > 10.0 * dry_tail,
        "wet=1 的尾窗 RMS 必须显著高于 wet=0 基线（实得 {:.1}×）",
        wet_tail / dry_tail.max(f64::MIN_POSITIVE)
    );
    assert_eq!(
        wet.stats.limiter_gain_reductions, 0,
        "尾窗读数必须整体低于母线限制器阈值 ⇒ 归因唯一"
    );
    // 每量子每节点**恰好一次**的电平发布：A + aux + master = 3 帧/量子。
    // `meter_frames + meter_dropped_frames` 是"应发布帧数"（队列容量不足时后者补上）。
    assert_eq!(
        wet.stats.meter_frames + wet.stats.meter_dropped_frames,
        3 * TAIL_QUANTA as u64,
        "三条可渲染节点每量子恰好一帧（处理型总线的电平帧被搬到转发阶段，⛔ 不重复计量）"
    );
}

/// B2：**EQ 在总线上也生效**。
///
/// 夹具音高 36（C2 ≈ 65.4 Hz）+ 总线 `eq_low_gain: +12 dB`（低架）。轨音量 −12 dB
/// 让提升之后仍低于母线限制器阈值（否则 RMS 差不能唯一归因给通道条）。
#[test]
fn b2_the_bus_eq_raises_the_low_band() {
    let note = Note::at(0, 1_920, 36, 40);
    let plain = send_project(note, 0.0, -12.0, Vec::new());
    let boosted = send_project(
        note,
        0.0,
        -12.0,
        vec![effect(&[("eq_low_gain", 12.0), ("eq_low_freq", 200.0)])],
    );
    assert_uses_buses(&boosted);
    let plain = render(&plain, SHORT_QUANTA);
    let boosted = render(&boosted, SHORT_QUANTA);

    // 只看**音符还在响**的窗口（0..1 s = 0..48,000 样本），尾巴不参与。
    let window = 0..48_000;
    let (plain_rms, plain_peak) = rms_peak(&plain.left[window.clone()]);
    let (boosted_rms, boosted_peak) = rms_peak(&boosted.left[window]);
    println!(
        "[#158/B2] 低频夹具（pitch 36 ≈ 65.4 Hz，轨音量 −12 dB）：无设备 RMS={:.9}（{:.3} dBFS，峰值 {:.6}）；\
         总线 EQ +12 dB RMS={:.9}（{:.3} dBFS，峰值 {:.6}）；差={:+.3} dB；\
         insert_strip_frames={}；limiter_gain_reductions={}",
        plain_rms,
        dbfs(plain_rms),
        plain_peak,
        boosted_rms,
        dbfs(boosted_rms),
        boosted_peak,
        dbfs(boosted_rms) - dbfs(plain_rms),
        boosted.stats.insert_strip_frames,
        boosted.stats.limiter_gain_reductions,
    );

    assert_eq!(
        plain.stats.insert_strip_frames, 0,
        "无设备的总线必须走快路（通道条计数 0）"
    );
    assert_eq!(
        boosted.stats.insert_strip_frames,
        (SHORT_QUANTA * 128) as u64,
        "总线通道条必须**恰好每量子处理一次**"
    );
    assert_eq!(
        boosted.stats.limiter_gain_reductions, 0,
        "提升臂必须整体低于母线限制器阈值 ⇒ RMS 差归因唯一"
    );
    assert!(
        boosted_peak < 0.9,
        "峰值 {boosted_peak:.6} 必须低于限制器阈值 0.9"
    );
    assert!(
        dbfs(boosted_rms) - dbfs(plain_rms) > 3.0,
        "总线上的 +12 dB 低架必须让低频夹具的 RMS 上升 > 3 dB（实得 {:+.3} dB）",
        dbfs(boosted_rms) - dbfs(plain_rms)
    );
    assert_ne!(
        plain.left_bits(),
        boosted.left_bits(),
        "总线 EQ 必须真的改变输出（逐位相同 ⇒ 链路没接上）"
    );
}

/// B3：**旁通生效** —— `bypassed = true` 的设备不武装 ⇒ 回到干信号。
///
/// 对照是"**没有设备**"的同一条工程：两者必须**逐位相同**（旁通不是"参数取成透明"，
/// 而是整台设备在构造期就被 `InsertParams::from_devices` 跳过）。
#[test]
fn b3_a_bypassed_bus_insert_returns_the_dry_signal() {
    let note = Note::at(0, 960, 48, 40);
    let mut bypassed_reverb = reverb_device(1.0, 0.8);
    bypassed_reverb.bypassed = true;
    let dry = send_project(note, 0.0, 0.0, Vec::new());
    let bypassed = send_project(note, 0.0, 0.0, vec![bypassed_reverb]);
    assert_uses_buses(&bypassed);
    let dry = render(&dry, SHORT_QUANTA);
    let bypassed = render(&bypassed, SHORT_QUANTA);

    let (dry_tail, _) = rms_peak(&dry.left[30_000..]);
    let (bypassed_tail, _) = rms_peak(&bypassed.left[30_000..]);
    println!(
        "[#158/B3] bypassed=true：fingerprint={:#018x}（无设备 {:#018x}）；\
         insert_strip/reverb/convolution={}/{}/{}；尾段 RMS={:.9}（无设备 {:.9}）",
        bypassed.fingerprint(),
        dry.fingerprint(),
        bypassed.stats.insert_strip_frames,
        bypassed.stats.insert_reverb_frames,
        bypassed.stats.insert_convolution_frames,
        bypassed_tail,
        dry_tail,
    );

    assert_eq!(
        dry.fingerprint(),
        bypassed.fingerprint(),
        "bypassed=true 必须与'没有设备'逐位相同"
    );
    assert_eq!(bypassed.stats.insert_strip_frames, 0);
    assert_eq!(bypassed.stats.insert_reverb_frames, 0);
    assert_eq!(bypassed.stats.insert_convolution_frames, 0);
    assert!(
        bypassed_tail < 1e-6,
        "旁通之后尾段必须回到干信号（静音）：{bypassed_tail:.9}"
    );
}

/// B4：**快路可判** —— 无设备的总线不进插入路径。
///
/// 两条腿：
/// 1. 无设备总线（硬声相 → L ≠ R）：三个插入计数为 0，且输出与**直连 master**
///    逐位相同（证明发送仍然只是"搬运"，没有被动过）；
/// 2. **正对照**：同一夹具只多挂一台湿混响 ⇒ 计数 > 0 且输出**必须不同**
///    （下混会改变硬声相的左右分配 —— 这正是裁决 (甲) 的代价，不是偶然）。
#[test]
fn b4_a_bus_without_devices_stays_on_the_fast_path() {
    let note = Note::at(0, 960, 48, 40);
    let direct = direct_project(note, -1.0);
    let fast = send_project(note, -1.0, 0.0, Vec::new());
    let armed = send_project(note, -1.0, 0.0, vec![reverb_device(1.0, 0.8)]);
    assert_uses_buses(&fast);
    let direct = render(&direct, SHORT_QUANTA);
    let fast = render(&fast, SHORT_QUANTA);
    let armed = render(&armed, SHORT_QUANTA);

    println!(
        "[#158/B4] 无设备总线：insert counts={}/{}/{}；fingerprint={:#018x}（直连 {:#018x}）；\
         挂设备：insert_reverb_frames={}；fingerprint={:#018x}",
        fast.stats.insert_strip_frames,
        fast.stats.insert_reverb_frames,
        fast.stats.insert_convolution_frames,
        fast.fingerprint(),
        direct.fingerprint(),
        armed.stats.insert_reverb_frames,
        armed.fingerprint(),
    );

    assert_eq!(fast.stats.insert_strip_frames, 0, "无设备 ⇒ 通道条快路");
    assert_eq!(fast.stats.insert_reverb_frames, 0, "无设备 ⇒ 混响快路");
    assert_eq!(fast.stats.insert_convolution_frames, 0, "无设备 ⇒ 卷积快路");
    assert_eq!(
        fast.fingerprint(),
        direct.fingerprint(),
        "无设备总线的输出必须与直连 master 逐位相同（发送只被搬运）"
    );

    // 正对照：设备真的武装之后，路径必须进、输出必须变。
    assert!(
        armed.stats.insert_reverb_frames > 0,
        "挂了湿混响的总线必须真的进插入路径（计数 > 0）"
    );
    assert_ne!(
        armed.fingerprint(),
        fast.fingerprint(),
        "总线插入必须改变输出（逐位相同 ⇒ 混响被接到了不参与混音的地方）"
    );
}

/// B6：**卷积混响也在总线上生效** —— 第三件器件。
///
/// 隔离方法：`conv_wet: 0` 的臂与"没有设备"逐位相同（器件 `is_active()` 为假 ⇒
/// 整段跳过）；`conv_wet: 1` 的臂在**同一条干路**上叠加湿路径 ⇒ `wet - dry` **恰好是**
/// 总线卷积的湿输出（下混与干路两者完全相同）。IR 长度固定 0.1 s（引擎常量，不是
/// 旋钮）⇒ 尾窗取音符结束后的 **0–100 ms**（`24_000..28_800`）：那里干路的 release
/// 还在响，所以用**差信号**而不是绝对 RMS 来隔离湿尾。
#[test]
fn b6_the_bus_convolution_tail_is_audible() {
    let note = Note::at(0, 960, 48, 40);
    let dry_project = send_project(note, 0.0, 0.0, Vec::new());
    let wet_project = send_project(
        note,
        0.0,
        0.0,
        // IR 增益 −24 dB：卷积的湿路径比干路响约 30 dB（IR 归一化后 4 800 点求和），
        // 不衰减就会把母线限制器压进来 ⇒ 差信号不再唯一归因给卷积。
        vec![effect(&[("conv_wet", 1.0), ("conv_ir_gain_db", -24.0)])],
    );
    assert_uses_buses(&wet_project);
    let dry = render(&dry_project, TAIL_QUANTA);
    let wet = render(&wet_project, TAIL_QUANTA);

    let window = 24_000..28_800;
    let difference: Vec<f32> = wet
        .left
        .iter()
        .zip(&dry.left)
        .map(|(wet_sample, dry_sample)| wet_sample - dry_sample)
        .collect();
    let (diff_rms, diff_peak) = rms_peak(&difference[window.clone()]);
    let (dry_rms, _) = rms_peak(&dry.left[window.clone()]);
    let (wet_rms, _) = rms_peak(&wet.left[window]);
    let expected_frames = (TAIL_QUANTA * 128) as u64;
    println!(
        "[#158/B6] 尾窗 [24000..28800)（音符结束后的 0–100 ms＝IR 长度）：\
         干臂 RMS={:.9}；湿臂 RMS={:.9}；差信号（纯卷积湿输出）RMS={:.9}、峰值={:.9}；\
         insert_convolution_frames={}（渲染帧数 {}）；insert_reverb/strip={}/{}；\
         limiter_gain_reductions={}",
        dry_rms,
        wet_rms,
        diff_rms,
        diff_peak,
        wet.stats.insert_convolution_frames,
        expected_frames,
        wet.stats.insert_reverb_frames,
        wet.stats.insert_strip_frames,
        wet.stats.limiter_gain_reductions,
    );

    assert_eq!(
        wet.stats.insert_convolution_frames, expected_frames,
        "总线卷积必须**恰好每量子处理一次**"
    );
    assert_eq!(wet.stats.insert_reverb_frames, 0, "本夹具没有混响设备");
    assert_eq!(wet.stats.insert_strip_frames, 0, "本夹具没有通道条设备");
    assert_eq!(
        wet.stats.limiter_gain_reductions, 0,
        "两条臂都必须低于母线限制器阈值 ⇒ 差信号归因唯一"
    );
    assert!(
        diff_rms > 1e-4,
        "差信号（纯湿输出）在 IR 尾窗内必须非零：{diff_rms:.9}"
    );
    assert!(diff_peak > 1e-3, "纯湿输出的峰值必须可判：{diff_peak:.9}");
    assert_ne!(dry.left_bits(), wet.left_bits(), "总线卷积必须真的改变输出");
}

/// B5：**尺寸红线** ＋ 裁决 (甲) 的代价读数。
///
/// 本片**没有**新增每总线状态：总线复用逐轨武装池里**同一台**器件（它的 `TrackV3`
/// 本来就在池里占一个槽位）⇒ `EngineRuntime` / `EngineSnapshot` 与 `#154` 之后
/// **逐字节相同**。这里同时打印三件器件的 `size_of`（"左右各一套"要各自再备 16 份
/// 的那三种类型），作为裁决里"缓存翻倍"的量化背景。
#[test]
fn b5_sizes_stay_under_the_192_kib_red_line() {
    let runtime = core::mem::size_of::<EngineRuntime>();
    let snapshot = core::mem::size_of::<EngineSnapshot>();
    let voice = core::mem::size_of::<SfzVoice>();
    let pool = core::mem::size_of::<SfzVoicePool>();
    let region = core::mem::size_of::<PreparedRegion>();
    let strip = core::mem::size_of::<ChannelStrip>();
    let reverb = core::mem::size_of::<Reverb>();
    let convolution = core::mem::size_of::<ConvolutionReverb>();
    println!(
        "[#158/B5] size_of: EngineRuntime={runtime} EngineSnapshot={snapshot} \
         SfzVoice={voice} SfzVoicePool={pool} PreparedRegion={region}；\
         器件（单槽内联部分）ChannelStrip={strip} Reverb={reverb} ConvolutionReverb={convolution}；\
         门槛 196608；余量={}",
        196_608 - runtime
    );
    assert_eq!(runtime, 180_144, "EngineRuntime 必须仍是 180144 B（＋0）");
    assert_eq!(snapshot, 4_576, "EngineSnapshot 必须仍是 4576 B（＋0）");
    assert_eq!(voice, 40, "SfzVoice 必须仍是 40 B");
    assert_eq!(pool, 1_712, "SfzVoicePool 必须仍是 1712 B");
    assert_eq!(region, 6_160, "PreparedRegion 必须仍是 6160 B");
    assert!(runtime < 196_608, "EngineRuntime 必须 < 192 KiB");
}
