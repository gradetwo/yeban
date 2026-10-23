//! ⭐ 任务单 `#261`：**实时引擎的逐边发送量槽位**（`SEND_GAIN_SLOT = 4`）。
//!
//! ## 这一条在补什么（`#258` 的勘察事实）
//!
//! `yeban_engine::param` 在 `#261` 之前只有四格（`TRACK_GAIN_SLOT` ／
//! `MASTER_GAIN_SLOT` ／ `TRACK_PAN_LEFT_SLOT` ／ `TRACK_PAN_RIGHT_SLOT`），
//! `4..=65535` 一律 `Unmapped` ⇒ `SendGain` 自动化**监听时不生效**（而离线导出已经生效），
//! 实测"开头比导出响 ≈ 45 dB"。本票给实时混音加一条**逐边**的发送量通路：
//!
//! * 地址是 `ParamAddress { entity: <路由边身份>, slot: SEND_GAIN_SLOT }`
//!   ⇒ 同一条轨的"发送边"与"直连边"是两条**独立**通路（`#258` 的否决判据）；
//! * 值是**线性绝对增益**（与声相同一口径）⇒ 音频线程只乘、**无除法**；
//! * 平滑器每个量子只推进**一次**（A/B 试听的第二分支走静态边增益）。
//!
//! ## 判据（与任务单 §3 的十条对应）
//!
//! | # | 判据 | 测试 |
//! | :-- | :--- | :--- |
//! | ② | 4 条阶梯事件 ⇒ 4 段严格递增、`peaks[3]/peaks[0] ≥ 3.0`、`armed_send_slot_count() == 1` | [`s2_a_four_step_ramp_on_the_send_edge`] |
//! | ③ | 不发任何事件 ⇒ 与**改前**逐位相同（冻结指纹） | [`s3_the_send_fixture_without_events_is_bit_identical_to_the_pre_change_engine`] |
//! | ④ | 事件值 = 静态边增益 ⇒ 与不发事件**逐位**相同 | [`s4_an_identity_event_is_bit_identical_to_sending_nothing`] |
//! | ⑤ | 跨边独立：只给发送边发事件 ⇒ 直连那一路 4 段峰值**逐位不变** | [`s5_a_send_ramp_does_not_touch_the_direct_route`] |
//! | ⑥ | 边数 > `SEND_SLOTS` ⇒ 计数 +1、不 panic、其它边不受影响 | [`s6_the_send_table_overflows_by_counting`] |
//! | — | 三个 `route_panned` 调用点各有一条覆盖：逐轨慢路（②）／总线转发（⑦）／声相平滑慢路（⑧） | [`s7_...`] / [`s8_...`] |
//! | — | 确定性：同一工程 + 同一事件序列 ⇒ 两次装配逐位相同 | [`s9_...`] |
//!
//! ## 边界（⛔ 不许当成已证）
//!
//! * ⛔ **没有人耳听**：判据证的是"发送量包络进了混音"（峰值/位模式），不是听感；
//! * ⛔ **A/B 试听的第二分支**没有被驱动（`audition_active` 没有公开面可以打开）
//!   ⇒ 裁决 6 的"只主线推进"由代码形状保证（`process_sends` 参数），本文件不假装测过；
//! * ⛔ 离线导出（`--export-wav` / MCP）走的是**另一条**执行路径
//!   （`yeban_render::automation::SendGainAutomation`）⇒ 本票**一位没改**它的字节。

mod support;

use std::str::FromStr as _;
use std::sync::Arc;

use support::NoteSpec;
use yeban_engine::meter::meter_channel;
use yeban_engine::param::{SEND_GAIN_SLOT, SEND_SLOTS};
use yeban_engine::ring::{EngineEvent, EventSender, ParamAddress, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, RetireQueue, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

/// 判据用的音符：480 tick = **12 000** 样本（120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本）。
const NOTE: NoteSpec = NoteSpec::at(0, 2_400, 60, 100);

/// 每一段阶梯的帧数（任务单 §3 的"各 12000 帧"）。
const STEP_FRAMES: usize = 12_000;

/// 四段阶梯的**绝对值**（裁决 2：线性绝对增益 ⇒ 最后一段是前一段的两倍）。
const STEPS: [f32; 4] = [0.125, 0.25, 0.5, 1.0];

/// 夹具里用到的固定身份（**必须是确定性的**：边键序决定路由顺序 ⇒ 决定求和顺序）。
///
/// ⚠ 这条不是洁癖：`EntityId::new()` 是随机 ULID，而 `RoutingPlan` 的出边顺序来自
/// `routing_graph.edges` 的 `BTreeMap` 键序 ⇒ 随机身份会让"逐位"判据在不同次运行里
/// 比不过。本文件所有身份都从同一个前缀派生。
fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{tail}")).expect("夹具 ULID 必须合法")
}

/// 一条测试边（**显式身份** —— 判据要直接给这条边发事件）。
struct EdgeSpec {
    id: EntityId,
    source: EntityId,
    destination: EntityId,
    kind: RoutingKind,
    gain_db: Option<f32>,
}

/// 组装工程：一条有音符的 MIDI 轨 ＋ 可选 aux ＋ 若干"幽灵"节点 ＋ 显式边表。
///
/// "幽灵"节点 = 出现在 `routing_graph.nodes` 里、但**不是可渲染音轨**的节点
/// （`RoutingGraph::validate` 只要求两端在 `nodes` 里 ⇒ 工程合法）。
/// 指向它的边会被实时侧**跳过**（`RoutingPlan::slot_of` 为 `None`）⇒ 那条发送
/// 没有可听的落点 —— 这正是判据 ⑤ 需要的"发送不影响直连"的隔离条件。
fn build_project(
    master: EntityId,
    track: EntityId,
    aux: Option<EntityId>,
    ghosts: &[EntityId],
    edges: &[EdgeSpec],
) -> YebanProjectV1 {
    let clip = id("K001");
    let placement = id("P001");
    let note_id = id("N001");

    let mut tracks = std::collections::BTreeMap::new();
    let mut lead = TrackV3 {
        id: track,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    lead.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: NOTE.start_tick,
            duration_ticks: NOTE.duration_ticks,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    tracks.insert(track, lead);
    if let Some(aux) = aux {
        tracks.insert(
            aux,
            TrackV3 {
                id: aux,
                name: "Aux".to_owned(),
                kind: TrackKind::AuxReturn,
                ..TrackV3::default()
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

    let mut clip_pool = std::collections::BTreeMap::new();
    clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi {
                notes: std::collections::BTreeMap::from([(
                    note_id,
                    MidiNote {
                        velocity: NOTE.velocity,
                        ..MidiNote::new(note_id, NOTE.start_tick, NOTE.pitch, NOTE.duration_ticks)
                    },
                )]),
            },
        },
    );

    let mut nodes = vec![track, master];
    if let Some(aux) = aux {
        nodes.push(aux);
    }
    nodes.extend_from_slice(ghosts);

    let mut routing = RoutingGraph {
        nodes,
        ..RoutingGraph::default()
    };
    for edge in edges {
        routing.edges.insert(
            edge.id,
            RoutingEdge {
                id: edge.id,
                source_node: edge.source,
                destination_node: edge.destination,
                kind: edge.kind,
                gain_db: edge.gain_db,
            },
        );
    }

    YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks,
        routing_graph: routing,
        clip_pool,
        ..YebanProjectV1::default()
    }
}

/// 发送量判据的夹具句柄。
struct SendFixture {
    project: YebanProjectV1,
    track: EntityId,
    send_edge: EntityId,
    return_edge: EntityId,
}

/// 可听的发送夹具：`track --send--> aux --return--> master`（两条边都搬音频）。
///
/// `send_db` / `return_db` 是两条边的**静态**边增益（`None` = 单位增益）。
fn audible_fixture(send_db: Option<f32>, return_db: Option<f32>) -> SendFixture {
    let master = id("M001");
    let track = id("T001");
    let aux = id("A001");
    let send_edge = id("E100");
    let return_edge = id("E200");
    let project = build_project(
        master,
        track,
        Some(aux),
        &[],
        &[
            EdgeSpec {
                id: send_edge,
                source: track,
                destination: aux,
                kind: RoutingKind::SendToAux,
                gain_db: send_db,
            },
            EdgeSpec {
                id: return_edge,
                source: aux,
                destination: master,
                kind: RoutingKind::BusToMaster,
                gain_db: return_db,
            },
        ],
    );
    SendFixture {
        project,
        track,
        send_edge,
        return_edge,
    }
}

/// 判据 ③ 的**改前**指纹（base 提交 `47239e6c` 上、同一夹具、同一量子数）。
///
/// ## 怎么量的（可复算）
///
/// 1. 同一份夹具代码（本文件 `audible_fixture(Some(-60.0), None)`）；
/// 2. 在**改前**的引擎上渲染 `375` 个量子（`375 × 128 = 48 000` 帧）；
/// 3. 对**全部样本的位模式**做 FNV-1a 64（`support::Render::fingerprint` 的同一算法，
///    左右两声道都在内）。
///
/// ⚠ 这个常数是"**没有 `SendGain` 事件的工程不得被本票改字节**"的机械证据：
/// ⭐ `#300` **修对、⛔ 回归**：中性中心 `0.707 → 1.0`（监视源侧 +3.0103 dB）⇒
/// 同一"没有 SendGain 事件"的夹具的 FNV 由 `0xe0b87258510f06dd` 变为
/// `0x02f55eafe3b4bba1`。
/// ⭐ `#302` **再一次修对、⛔ 回归**：默认档去掉母线限制器的 33 帧环 ⇒ 时序前移
/// 33 帧 ⇒ FNV 变为 `0x6225f9d56eb8a48d`；峰值不变（`0.000774794`）。
/// 夹具与判据本身一字未改：它仍钉"没有 SendGain 事件 ⇒ 与基线逐位相同"。
/// 若这条变红 ⇒ 要么发送量路径被误动，要么默认档限制器/时序回归。
const PRE_CHANGE_SEND_FIXTURE_FNV: u64 = 0x6225_f9d5_6eb8_a48d;

/// 事件驱动的渲染夹具（保留事件生产端 ⇒ 可以在量子之间真的发 `SetParam`）。
///
/// ⚠ `slot` / `queue` 在本文件里**没有读点**，但必须保活：前者是快照槽的所有者
/// （`SnapshotReader` 只持 `Arc`），后者是退役回收的消费端。丢掉它们不会立刻出错，
/// 但那就不是"与生产同一条装配"了 —— 判据要跑在产品路径上。
#[allow(dead_code)]
struct SendRig {
    slot: Arc<SnapshotSlot>,
    queue: RetireQueue,
    sender: EventSender,
    runtime: EngineRuntime,
    output: Vec<f32>,
    left: Vec<f32>,
    right: Vec<f32>,
}

impl SendRig {
    fn new(project: &YebanProjectV1) -> Self {
        let snapshot = EngineSnapshot::from_project(project, 1).expect("夹具必须能编译成快照");
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
            output: Vec::new(),
            left: Vec::new(),
            right: Vec::new(),
        }
    }

    /// 发一条 `SetParam`（返回真的写进 SPSC 的条数）。
    fn set_param(&mut self, entity: EntityId, slot: u16, value: f32) -> usize {
        self.sender.publish(&[EngineEvent::SetParam {
            target: ParamAddress::new(entity, slot),
            value,
        }])
    }

    /// 渲染**恰好** `frames` 帧（`process_quantum` 会按 128 帧切块、余数单独一块）。
    fn frames(&mut self, frames: usize) {
        self.output.clear();
        self.output.resize(frames * 2, 0.0);
        self.runtime.process_quantum(&mut self.output, 2);
        for chunk in self.output.as_chunks::<2>().0 {
            self.left.push(chunk[0]);
            self.right.push(chunk[1]);
        }
    }

    /// `[start, end)` 这段样本的峰值（绝对值最大）。
    fn peak_since(&self, start: usize) -> f32 {
        self.left[start..]
            .iter()
            .chain(self.right[start..].iter())
            .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
    }

    /// 全部样本位模式的 FNV-1a 64（与 `support::Render::fingerprint` 同一算法）。
    fn fingerprint(&self) -> u64 {
        let mut hash = 0xcbf2_9ce4_8422_2325u64;
        for sample in self.left.iter().chain(self.right.iter()) {
            for byte in sample.to_bits().to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        hash
    }

    /// 逐位比较两条声道的辅助读数：`(左, 右)` 的位模式。
    fn bits(&self) -> (Vec<u32>, Vec<u32>) {
        (
            self.left.iter().map(|sample| sample.to_bits()).collect(),
            self.right.iter().map(|sample| sample.to_bits()).collect(),
        )
    }
}

/// 判据 ②︙**渐强**：4 条阶梯 `SetParam { edge, 4, … }`（各 12 000 帧）
/// ⇒ 4 段峰值**严格递增**、`peaks[3]/peaks[0] ≥ 3.0`、`armed_send_slot_count() == 1`。
///
/// ⚠ `armed_send_slot_count()` **读的是武装表**（`armed_send_armed` 里为真的行数），
/// ⛔ 不是从输出反推：夹具里有 **2** 条可寻址音频边（发送 ＋ 返回），只有被事件
/// 打到的那一条为真 ⇒ 1。
#[test]
fn s2_a_four_step_ramp_on_the_send_edge() {
    // 静态边增益取 -60 dB（≈ 0.001）：阶梯全部从它**之上**起步 ⇒ 每段峰值由阶梯
    // 目标主导，第一段的起点不会污染比值。
    let fixture = audible_fixture(Some(-60.0), None);
    let mut rig = SendRig::new(&fixture.project);
    // 第一个量子：武装快照（发送量表建起来，但**没有**一条被自动化接管）。
    rig.frames(128);
    assert_eq!(
        rig.runtime.armed_send_table_len(),
        2,
        "两条音频边（发送 ＋ 返回）都必须进武装表"
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        0,
        "还没有事件 ⇒ 没有一条边被自动化接管"
    );

    let mut peaks = Vec::new();
    for value in STEPS {
        assert_eq!(
            rig.set_param(fixture.send_edge, SEND_GAIN_SLOT, value),
            1,
            "每条事件都必须进 SPSC"
        );
        let start = rig.left.len();
        rig.frames(STEP_FRAMES);
        peaks.push(rig.peak_since(start));
    }
    println!(
        "[#261/②] send 边阶梯 {:?} ⇒ 四段峰值 {:?}；比值 {:.6}；armed_send_slot_count={}；\
         send_automation_frames={}；unmapped={}；rejects={}",
        STEPS,
        peaks,
        peaks[3] / peaks[0],
        rig.runtime.armed_send_slot_count(),
        rig.runtime.send_automation_frames(),
        rig.runtime.stats().param_unmapped_events,
        rig.runtime.send_automation_value_rejects()
    );
    assert!(peaks[0] > 1.0e-4, "第一段必须真的出声（实得 {}）", peaks[0]);
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "四段峰值必须严格递增（实得 {peaks:?}）"
    );
    assert!(
        peaks[3] / peaks[0] >= 3.0,
        "末段/首段必须 ≥ 3.0（实得 {:.6}）",
        peaks[3] / peaks[0]
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        1,
        "只给发送那条边发事件 ⇒ 武装表里只有一格为真"
    );
    assert_eq!(
        rig.runtime.armed_send_target(&fixture.send_edge),
        Some(1.0),
        "目标值读出来必须等于最后一条事件"
    );
    assert_eq!(
        rig.runtime.armed_send_target(&fixture.return_edge),
        Some(1.0),
        "返回边**没有**收到事件 ⇒ 目标仍是它的静态增益（1.0）"
    );
    assert_eq!(
        rig.runtime.stats().param_unmapped_events,
        0,
        "槽位 4 必须被事件循环截走（⛔ 不许落进 ParamTable 判未映射）"
    );
    assert_eq!(
        rig.runtime.send_automation_value_rejects(),
        0,
        "四个值都合法 ⇒ 没有拒绝"
    );
    assert_eq!(
        rig.runtime.send_automation_frames(),
        (STEP_FRAMES * STEPS.len()) as u64,
        "平滑路径被乘过的帧数 = 四段之和（每帧恰好一次）"
    );
}

/// 判据 ③︙**正对照**：不发任何事件 ⇒ 与**改前**逐位相同（冻结指纹 ＋ 表读数为证）。
#[test]
fn s3_the_send_fixture_without_events_is_bit_identical_to_the_post_302_engine() {
    let fixture = audible_fixture(Some(-60.0), None);
    let rendered = support::render(&fixture.project, 375);
    let mut rig = SendRig::new(&fixture.project);
    rig.frames(375 * 128);
    println!(
        "[#261/③] 无事件指纹 support={:#018x} / rig={:#018x}（#302 重钉基准 {:#018x}）；峰值 {:.9}",
        rendered.fingerprint(),
        rig.fingerprint(),
        PRE_CHANGE_SEND_FIXTURE_FNV,
        rendered.peak()
    );
    assert_eq!(
        rig.fingerprint(),
        rendered.fingerprint(),
        "两条驱动路径（support::render / 本地 rig）必须同解"
    );
    assert_eq!(
        rendered.fingerprint(),
        PRE_CHANGE_SEND_FIXTURE_FNV,
        "没有 SendGain 事件的工程必须与 #302 重钉基准逐位相同"
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        0,
        "没有事件 ⇒ 武装表里没有一条边被接管（平滑路径一位没进）"
    );
    assert_eq!(
        rig.runtime.send_automation_frames(),
        0,
        "没有事件 ⇒ 被平滑发送量乘过的帧数是 0"
    );
    for (edge, static_gain) in rig.runtime.armed_send_edges() {
        assert_eq!(
            rig.runtime.armed_send_target(edge),
            Some(*static_gain),
            "未武装的槽位，平滑器目标必须停在静态边增益（恒等点）"
        );
    }
}

/// 判据 ④︙**恒等**：事件值 = 静态边增益 ⇒ 与不发事件**逐位**相同（⛔ 不是容差）。
///
/// 两种口径都覆盖：
/// * (a) 静态 = **单位增益** ⇒ 基线走"快路"（`sum_into_bus`），事件打开慢路；
/// * (b) 静态 = **-6.0206 dB**（线性 `0.5`）⇒ 两者都走慢路。
#[test]
fn s4_an_identity_event_is_bit_identical_to_sending_nothing() {
    // --- (a) 单位增益：静态 None ⇒ 1.0，事件值 1.0 ---
    let fixture = audible_fixture(None, None);
    let baseline = support::render(&fixture.project, 375);
    let mut rig = SendRig::new(&fixture.project);
    rig.frames(128);
    assert_eq!(rig.set_param(fixture.send_edge, SEND_GAIN_SLOT, 1.0), 1);
    rig.frames(375 * 128 - 128);
    println!(
        "[#261/④a] 单位增益恒等：baseline {:#018x} / with_event {:#018x}；armed={}",
        baseline.fingerprint(),
        rig.fingerprint(),
        rig.runtime.armed_send_slot_count()
    );
    assert_eq!(
        rig.fingerprint(),
        baseline.fingerprint(),
        "事件值 == 静态边增益（1.0）⇒ 必须与不发事件逐位相同"
    );

    // --- (b) 非单位静态增益：静态 -6.0206 dB ⇒ 0.5，事件值 0.5 ---
    let fixture = audible_fixture(Some(-6.020_6), None);
    let baseline = support::render(&fixture.project, 375);
    let mut rig = SendRig::new(&fixture.project);
    rig.frames(128);
    assert_eq!(rig.set_param(fixture.send_edge, SEND_GAIN_SLOT, 0.5), 1);
    rig.frames(375 * 128 - 128);
    println!(
        "[#261/④b] 0.5 恒等：baseline {:#018x} / with_event {:#018x}；armed={}",
        baseline.fingerprint(),
        rig.fingerprint(),
        rig.runtime.armed_send_slot_count()
    );
    assert_eq!(
        rig.fingerprint(),
        baseline.fingerprint(),
        "事件值 == 静态边增益（0.5）⇒ 必须与不发事件逐位相同"
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        1,
        "恒等事件仍然**武装**了那条边（恒等是值相等，不是没生效）"
    );
}

/// 判据 ⑤︙⭐ **跨边独立（最能否决"轨道级近似"的一条）**。
///
/// 夹具：同一条轨**发送 ＋ 直连并存**（发送边指向一个**没有渲染落点**的幽灵节点
/// ⇒ 母线上只有直连那一路）。只给**发送**那条边发 4 段阶梯 ⇒ 直连那一路的
/// 4 段峰值（以及**全部样本的位模式**）必须**逐位不变**。
///
/// ⚠ 任何"按轨施加发送量"的近似都会把直连那一路一起缩放 ⇒ 这条判据必红。
#[test]
fn s5_a_send_ramp_does_not_touch_the_direct_route() {
    let master = id("M001");
    let track = id("T001");
    let ghost = id("G900");
    let send_edge = id("E100");
    let direct_edge = id("E300");
    let project = build_project(
        master,
        track,
        None,
        &[ghost],
        &[
            EdgeSpec {
                id: send_edge,
                source: track,
                destination: ghost,
                kind: RoutingKind::SendToAux,
                gain_db: Some(-6.020_6),
            },
            EdgeSpec {
                id: direct_edge,
                source: track,
                destination: master,
                kind: RoutingKind::TrackToBus,
                gain_db: None,
            },
        ],
    );

    // --- 基线：不发事件 ---
    let mut baseline = SendRig::new(&project);
    baseline.frames(128);
    assert_eq!(baseline.runtime.armed_send_table_len(), 2);
    let mut baseline_peaks = Vec::new();
    for _ in STEPS {
        let start = baseline.left.len();
        baseline.frames(STEP_FRAMES);
        baseline_peaks.push(baseline.peak_since(start));
    }

    // --- 事件：只给**发送**那条边发阶梯 ---
    let mut rig = SendRig::new(&project);
    rig.frames(128);
    let mut peaks = Vec::new();
    for value in STEPS {
        assert_eq!(rig.set_param(send_edge, SEND_GAIN_SLOT, value), 1);
        let start = rig.left.len();
        rig.frames(STEP_FRAMES);
        peaks.push(rig.peak_since(start));
    }
    println!(
        "[#261/⑤] 只给发送边发事件：直连那一路四段峰值 基线={baseline_peaks:?} 事件={peaks:?}；\
         指纹 基线={:#018x} 事件={:#018x}；armed_send_slot_count={}",
        baseline.fingerprint(),
        rig.fingerprint(),
        rig.runtime.armed_send_slot_count()
    );
    assert!(baseline_peaks[0] > 1.0e-3, "直连那一路必须真的出声");
    for index in 0..STEPS.len() {
        assert_eq!(
            peaks[index].to_bits(),
            baseline_peaks[index].to_bits(),
            "第 {index} 段：直连那一路的峰值必须逐位不变（实得 {} vs {}）",
            peaks[index],
            baseline_peaks[index]
        );
    }
    assert_eq!(
        rig.fingerprint(),
        baseline.fingerprint(),
        "整条母线的样本位模式必须逐位相同（发送那条边没有可听落点）"
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        1,
        "只有发送那条边被自动化接管（直连那条边一位没动）"
    );
    assert_eq!(
        rig.runtime.stats().param_unmapped_events,
        0,
        "发送事件必须被截走（⛔ 不落进 ParamTable）"
    );
}

/// 判据 ⑥︙**容量**：边数 > `SEND_SLOTS` ⇒ 计数 +1、⛔ 不 panic、其它边不受影响。
///
/// 夹具：一条轨 + **65** 条出边（指向 65 个幽灵节点）。表的确定性顺序由边键序决定
/// ⇒ 前 `SEND_SLOTS` 条武装、剩下的逐条计数。
#[test]
fn s6_the_send_table_overflows_by_counting() {
    let master = id("M001");
    let track = id("T001");
    let edge_count = SEND_SLOTS + 1;
    let ghosts: Vec<EntityId> = (0..edge_count)
        .map(|index| id(&format!("G{index:03}")))
        .collect();
    let mut edges = Vec::with_capacity(edge_count);
    let mut edge_ids = Vec::with_capacity(edge_count);
    for (index, ghost) in ghosts.iter().enumerate() {
        let edge_id = id(&format!("E{index:03}"));
        edge_ids.push(edge_id);
        edges.push(EdgeSpec {
            id: edge_id,
            source: track,
            destination: *ghost,
            kind: RoutingKind::SendToAux,
            gain_db: None,
        });
    }
    let project = build_project(master, track, None, &ghosts, &edges);

    let mut rig = SendRig::new(&project);
    rig.frames(128);
    println!(
        "[#261/⑥] 边数={edge_count} ⇒ armed_send_table_len={} capacity_drops={}",
        rig.runtime.armed_send_table_len(),
        rig.runtime.send_capacity_drops()
    );
    assert_eq!(
        rig.runtime.armed_send_table_len(),
        SEND_SLOTS,
        "表最多装 {SEND_SLOTS} 条边"
    );
    assert_eq!(
        rig.runtime.send_capacity_drops(),
        1,
        "第 {} 条边必须**计数**（⛔ 不 panic、⛔ 不静默）",
        SEND_SLOTS + 1
    );

    let armed: Vec<EntityId> = rig
        .runtime
        .armed_send_edges()
        .iter()
        .map(|(id, _)| *id)
        .collect();
    let dropped = edge_ids
        .iter()
        .copied()
        .find(|edge| !armed.contains(edge))
        .expect("恰好一条边装不下");
    let kept = armed[0];

    // 装下的那条边：正常生效，不产生未映射读数。
    assert_eq!(rig.set_param(kept, SEND_GAIN_SLOT, 0.25), 1);
    rig.frames(128);
    assert_eq!(rig.runtime.armed_send_target(&kept), Some(0.25));
    assert_eq!(rig.runtime.armed_send_slot_count(), 1);
    let unmapped_before = rig.runtime.stats().param_unmapped_events;
    assert_eq!(unmapped_before, 0);

    // 装不下的那条边：保留**静态**边增益（读数仍是 1.0、未武装），事件交回
    // `ParamTable` 判 `Unmapped` 并计数（单一归属）。
    assert_eq!(rig.runtime.armed_send_target(&dropped), None);
    assert_eq!(rig.set_param(dropped, SEND_GAIN_SLOT, 0.25), 1);
    rig.frames(128);
    assert_eq!(
        rig.runtime.stats().param_unmapped_events,
        unmapped_before + 1,
        "装不下的边 ⇒ 事件必须被计数为未映射（⛔ 不静默丢弃）"
    );
    assert_eq!(
        rig.runtime.armed_send_slot_count(),
        1,
        "装不下的边不得挤掉已武装的那条"
    );
    assert_eq!(
        rig.runtime.armed_send_target(&kept),
        Some(0.25),
        "其它边不受影响"
    );
}

/// ⑦︙第三个 `route_panned` 调用点（**总线转发 3a⁗**）：给 `aux → master` 这条
/// **返回边**发 4 段阶梯 ⇒ 同样严格递增。
///
/// 这一条与 ② 合起来覆盖"三处调用点"里的两处（逐轨慢路 ＋ 总线转发）。
#[test]
fn s7_a_four_step_ramp_on_the_bus_return_edge() {
    let fixture = audible_fixture(None, Some(-60.0));
    let mut rig = SendRig::new(&fixture.project);
    rig.frames(128);
    let mut peaks = Vec::new();
    for value in STEPS {
        assert_eq!(rig.set_param(fixture.return_edge, SEND_GAIN_SLOT, value), 1);
        let start = rig.left.len();
        rig.frames(STEP_FRAMES);
        peaks.push(rig.peak_since(start));
    }
    println!(
        "[#261/⑦] 返回边阶梯 {:?} ⇒ 四段峰值 {:?}；比值 {:.6}；armed_send_slot_count={}",
        STEPS,
        peaks,
        peaks[3] / peaks[0],
        rig.runtime.armed_send_slot_count()
    );
    assert!(peaks[0] > 1.0e-4, "第一段必须真的出声（实得 {}）", peaks[0]);
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "总线转发那一路也必须严格递增（实得 {peaks:?}）"
    );
    assert!(
        peaks[3] / peaks[0] >= 3.0,
        "末段/首段必须 ≥ 3.0（实得 {:.6}）",
        peaks[3] / peaks[0]
    );
    assert_eq!(rig.runtime.armed_send_slot_count(), 1);
}

/// ⑧︙第一个 `route_panned` 调用点里**声相平滑**那一支：声相自动化武装之后
/// 发送量阶梯仍然生效（两条平滑器各推进一次，不互相吃掉）。
#[test]
fn s8_a_send_ramp_still_works_while_pan_is_automated() {
    use yeban_engine::param::{TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT};

    let fixture = audible_fixture(Some(-60.0), None);
    let mut rig = SendRig::new(&fixture.project);
    rig.frames(128);
    // 声相自动化：硬左（绝对增益 L = 1.0、R = 0.0）⇒ 走声相的平滑路径。
    assert_eq!(rig.set_param(fixture.track, TRACK_PAN_LEFT_SLOT, 1.0), 1);
    assert_eq!(rig.set_param(fixture.track, TRACK_PAN_RIGHT_SLOT, 0.0), 1);
    rig.frames(128);
    assert!(
        rig.runtime.pan_automation_frames() > 0,
        "声相平滑路径必须走到"
    );

    let mut peaks = Vec::new();
    for value in STEPS {
        assert_eq!(rig.set_param(fixture.send_edge, SEND_GAIN_SLOT, value), 1);
        let start = rig.left.len();
        rig.frames(STEP_FRAMES);
        peaks.push(rig.peak_since(start));
    }
    println!(
        "[#261/⑧] 声相已自动化时的发送量阶梯 {:?} ⇒ 四段峰值 {:?}；比值 {:.6}；\
         pan_automation_frames={} send_automation_frames={}",
        STEPS,
        peaks,
        peaks[3] / peaks[0],
        rig.runtime.pan_automation_frames(),
        rig.runtime.send_automation_frames()
    );
    assert!(
        peaks[0] < peaks[1] && peaks[1] < peaks[2] && peaks[2] < peaks[3],
        "声相平滑与发送量平滑必须**各自**生效（实得 {peaks:?}）"
    );
    assert!(
        peaks[3] / peaks[0] >= 3.0,
        "发送量的比值不应被声相平滑吃掉（实得 {:.6}）",
        peaks[3] / peaks[0]
    );
    assert!(rig.runtime.send_automation_frames() > 0);
    assert_eq!(
        rig.runtime.stats().param_unmapped_events,
        0,
        "声相与发送量两条槽位都被截走"
    );
}

/// ⑨︙**确定性**：同一工程 + 同一事件序列 ⇒ 两次独立装配逐位相同。
#[test]
fn s9_the_send_automation_is_deterministic() {
    let fixture = audible_fixture(Some(-6.020_6), None);
    let run = || {
        let mut rig = SendRig::new(&fixture.project);
        rig.frames(128);
        for value in STEPS {
            assert_eq!(rig.set_param(fixture.send_edge, SEND_GAIN_SLOT, value), 1);
            rig.frames(STEP_FRAMES / 3);
        }
        (
            rig.fingerprint(),
            rig.bits(),
            rig.runtime.send_automation_frames(),
        )
    };
    let first = run();
    let second = run();
    println!(
        "[#261/⑨] 两次装配指纹 {:#018x} / {:#018x}；frames={}",
        first.0, second.0, first.2
    );
    assert_eq!(first.0, second.0, "同一输入必须给出逐位相同的输出");
    assert_eq!(first.1, second.1, "逐样本位模式必须相同");
    assert_eq!(first.2, second.2, "被平滑的帧数也必须相同");
}
