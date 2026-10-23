//! ⭐ 任务单 `#300` 的**正向判据**：中性声相下**监听 = 导出**（逐位）。
//!
//! ## 修的是什么（`#299` 量到的既有缺陷）
//!
//! 逐字事实（`#299` 报告 §5）：
//!
//! > **`pan == 0` 时、引擎源侧仍是 `0.707/0.707`、而离线是 `1/1` ⇒ 所以「监听 ≠ 导出」
//! > 在中性声相上就已经存在、⛔ 本单引入**
//!
//! `#299` 把差量成一个数（监听 `0.548988640` / 导出 `0.776387155`、ratio `√2/2`、
//! −3.0103 dB）并登记为下一票；`#300` 按人类裁决把引擎源侧的中心口径改成
//! `centered_pan_gains`（`pan == 0` ⇒ 逐位 `(1, 1)`，与离线同一条）⇒ 本文件现在钉的是
//! **修好之后的事实**：ratio = `1.000000`。
//!
//! ## 量什么
//!
//! 同一个中性 fixture（单轨、`pan = 0`、`volume_db = 0`、无设备、无自动化、
//! 主总线 `pan = 0`）：
//!
//! - **监听**：`EngineRuntime::process_quantum` 推 375 个量子 = 48 000 帧；
//! - **导出**：`render_project_f32_with_assets_under`（app 的离线渲染，引擎音源）；
//! - 读数：左右各自的**峰值绝对值**（单位：f32 满幅）与 `engine / offline` 的比。
//!
//! 期望：两侧都 L == R（居中），比值 = **`1.000000`**（浮点差 ≤ `1e-6`）。
//! ⚠ 边界：`pan != 0` 的轨不受本单影响（中心口径只改 `pan == 0`），逐位读数在
//! `yeban-engine/tests/bus_pan_mute_solo.rs` 的 B4／B5（`pan = -1.0 / -0.5`）里原样通过。

use std::collections::BTreeMap;

use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets_under;
use yeban_engine::meter::meter_channel;
use yeban_engine::ring::event_channel;
use yeban_engine::rt::EngineRuntime;
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_model::{
    ClipContent, ClipPlacement, ClipPoolEntry, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingGraph, RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};

const FRAMES: usize = 48_000;
const QUANTA: usize = FRAMES / 128;
/// 修好之后中性声相下监听/导出的**唯一**期望比：`1.0`（逐位相等；容差 `1e-6`）。
const UNITY_RATIO: f64 = 1.0;
/// `pan = -0.5` 的监听输出逐位指纹（FNV-1a 64）—— **`#302` 之后**实测的基线。
///
/// ⚠ 与 `#300` 的 `0xc5dd6500ff6831f8` 不同：`#302` 把母线限制器的默认档改成
/// **关**（监听整段旁通 33 帧环）⇒ 同一段音频的**时序前移 33 帧**，指纹必然变；
/// 峰值不变（`0.717288196 / 0.297110528`）。这是**修对**，不是回归。
const PAN_HALF_MONITORING_FNV: u64 = 0xbf50_497c_599f_f028;

/// 单轨中性 fixture：`pan = 0`、`volume_db = 0`、无设备、无自动化。
fn neutral_project() -> YebanProjectV1 {
    let master = EntityId::new();
    let lead = EntityId::new();
    let clip = EntityId::new();
    let placement = EntityId::new();
    let note = EntityId::new();

    let mut pool = BTreeMap::new();
    pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    note,
                    MidiNote {
                        velocity: 100,
                        ..MidiNote::new(note, 0, 69, 480)
                    },
                )]),
            },
        },
    );
    let mut lead_track = TrackV3 {
        id: lead,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        ..TrackV3::default()
    };
    lead_track.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 480,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    let master_track = TrackV3 {
        id: master,
        name: "Master".to_owned(),
        kind: TrackKind::Master,
        ..TrackV3::default()
    };
    let mut project = YebanProjectV1 {
        bpm: 120.0,
        master_bus_track_id: master,
        tracks: BTreeMap::from([(master, master_track), (lead, lead_track)]),
        clip_pool: pool,
        ..YebanProjectV1::default()
    };
    project.routing_graph = RoutingGraph {
        nodes: vec![lead, master],
        ..RoutingGraph::default()
    };
    let edge = EntityId::new();
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: lead,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    project.validate().expect("中性夹具必须合法");
    project
}

/// 监听腿：`EngineRuntime` 推 `QUANTA` 个量子，返回 `(左, 右)`。
fn render_monitoring(project: &YebanProjectV1) -> (Vec<f32>, Vec<f32>) {
    let snapshot = EngineSnapshot::from_project(project, 1).expect("快照");
    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (_sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);
    let mut left = Vec::with_capacity(FRAMES);
    let mut right = Vec::with_capacity(FRAMES);
    let mut out = vec![0.0f32; 256];
    for _ in 0..QUANTA {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
            right.push(out[frame * 2 + 1]);
        }
    }
    (left, right)
}

/// 导出腿：app 的离线 f32 渲染（引擎音源），拆成交织的左右。
fn render_export(project: &YebanProjectV1) -> (Vec<f32>, Vec<f32>) {
    let options = WavExportOptions {
        frames: FRAMES as u64,
        ..WavExportOptions::default()
    };
    let output = render_project_f32_with_assets_under(project, options, None, &[], None)
        .expect("中性夹具必须能导出");
    let mut left = Vec::with_capacity(FRAMES);
    let mut right = Vec::with_capacity(FRAMES);
    for pair in output.samples.as_chunks::<2>().0 {
        left.push(pair[0]);
        right.push(pair[1]);
    }
    (left, right)
}

fn peak(channel: &[f32]) -> f32 {
    channel
        .iter()
        .fold(0.0f32, |peak, sample| peak.max(sample.abs()))
}

/// ⭐ 核心判据：中性声相下监听 **=** 导出（`ratio = 1.000000`，浮点差 ≤ `1e-6`）。
///
/// 修前（`#299`）这里是 `0.707106805`（−3.0103 dB）；`#300` 把源侧中心口径对齐到
/// 离线之后，两侧**同一份**样本，ratio 逐位 `1.000000000`。
#[test]
fn monitoring_matches_export_at_neutral_pan() {
    let project = neutral_project();
    let (mon_l, mon_r) = render_monitoring(&project);
    let (exp_l, exp_r) = render_export(&project);

    let (mon_peak_l, mon_peak_r) = (peak(&mon_l), peak(&mon_r));
    let (exp_peak_l, exp_peak_r) = (peak(&exp_l), peak(&exp_r));
    let ratio_l = f64::from(mon_peak_l) / f64::from(exp_peak_l);
    let ratio_r = f64::from(mon_peak_r) / f64::from(exp_peak_r);

    println!(
        "[#300/neutral-leg] 监听 L/R = {mon_peak_l:.9} / {mon_peak_r:.9}；\
         导出 L/R = {exp_peak_l:.9} / {exp_peak_r:.9}；\
         ratio L/R = {ratio_l:.9} / {ratio_r:.9}（期望 {UNITY_RATIO:.9}，修前 0.707106805）"
    );

    assert!(mon_peak_l > 1e-3, "监听必须真的出声（否则比值是真空的）");
    assert!(exp_peak_l > 1e-3, "导出必须真的出声");
    assert!(
        (mon_peak_l - mon_peak_r).abs() <= 1e-9,
        "监听居中必须左右相等：{mon_peak_l} vs {mon_peak_r}"
    );
    assert!(
        (exp_peak_l - exp_peak_r).abs() <= 1e-9,
        "导出居中必须左右相等：{exp_peak_l} vs {exp_peak_r}"
    );
    // 修好之后的事实：监听与导出在**中性声相**上逐位同量（容差 1e-6）。
    // ⚠ 若有人把源侧中心口径改回 `0.707`，这条会变红 —— 那正是它要抓的回归。
    assert!(
        (ratio_l - UNITY_RATIO).abs() < 1e-6 && (ratio_r - UNITY_RATIO).abs() < 1e-6,
        "中性声相下监听/导出的比必须是 1.0（监听对齐导出）；实测 {ratio_l} / {ratio_r}"
    );
    // 顺带钉住"两侧峰值都 < 1.0 ⇒ 中性夹具不削波"（`#300` 的响度变化读数）。
    assert!(
        mon_peak_l < 1.0 && exp_peak_l < 1.0,
        "中性夹具不得削波：监听 {mon_peak_l} / 导出 {exp_peak_l}"
    );
}

/// 逐位指纹（FNV-1a 64，按 `f32::to_bits` 的小端字节；左声道全部在前、右声道在后）。
fn fnv1a_monitoring(left: &[f32], right: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for sample in left.iter().chain(right.iter()) {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// ⭐ `#300` 的**范围守卫**（`#302` 重钉）：中心口径只改 `pan == 0`。
///
/// 同一 fixture 只把 Lead 轨的 `pan` 从 `0.0` 改成 `-0.5` ⇒ 走的是**同一个**
/// `pan_gains(pan, law)` 调用（`centered_pan_gains` 对 `pan != 0` 原样转发）。
///
/// ⚠ 基线在 `#302` 之后重钉：默认档去掉 33 帧限制器环 ⇒ 时序前移 33 帧 ⇒
/// 指纹由 `0xc5dd6500ff6831f8` 变为 [`PAN_HALF_MONITORING_FNV`]；**峰值未变**
/// （`0.717288196 / 0.297110528`）⇒ 声相曲线本身没有被改。这是**修对**。
/// 若有人把中心规则扩散到 `pan != 0`（例如无条件 `(1, 1)`），这条仍会红。
#[test]
fn pan_off_centre_keeps_the_monitoring_gain_readings_after_the_302_re_pin() {
    let mut project = neutral_project();
    for track in project.tracks.values_mut() {
        if track.kind != TrackKind::Master {
            track.pan = -0.5;
        }
    }
    let (left, right) = render_monitoring(&project);
    let fingerprint = fnv1a_monitoring(&left, &right);
    println!(
        "[#300/pan-half] 监听 L/R 峰值 = {:.9} / {:.9}；指纹 = {fingerprint:#018x}（修前基线 {PAN_HALF_MONITORING_FNV:#018x}）",
        peak(&left),
        peak(&right)
    );
    assert_eq!(
        fingerprint, PAN_HALF_MONITORING_FNV,
        "pan = -0.5 的监听读数必须与 `#300` 之前逐位相同（中心口径只影响居中）"
    );
}
