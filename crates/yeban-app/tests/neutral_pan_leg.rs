//! ⭐ 任务单 `#299` 的**边界读数**：中性声相下「监听 ≠ 导出」的 **−3 dB**。
//!
//! ## 这条判据钉住的是一个**既有缺陷**，不是本单的修复
//!
//! 逐字事实（本单勘察，`#299` 报告 §5）：
//!
//! > **`pan == 0` 时、引擎源侧仍是 `0.707/0.707`、而离线是 `1/1` ⇒ 所以「监听 ≠ 导出」
//! > 在中性声相上就已经存在、⛔ 本单引入**
//!
//! 也就是说：**一轨摆在正中、监听比导出低 3 dB**。`#299` 只对齐总线的
//! pan／mute／solo，⛔ 不碰源侧声相（碰了会改中性总线的逐位基线，也超出本单）。
//! 本文件把这个差**量成一个数**，并把它登记为**下一票**；未来那一票修好时，
//! 本判据的期望值必须跟着改（它现在的绿＝"缺陷仍在"）。
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
//! 期望：两侧都 L == R（居中），比值 = `√2/2 = 0.70710678`（−3.0103 dB）。

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
const CENTRE_GAIN: f64 = core::f64::consts::FRAC_1_SQRT_2;

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

/// ⭐ 边界判据：中性声相下监听比导出**低 `√2/2`**（既有缺陷，见模块文档）。
#[test]
fn neutral_monitoring_is_minus_3db_against_export_until_the_next_ticket() {
    let project = neutral_project();
    let (mon_l, mon_r) = render_monitoring(&project);
    let (exp_l, exp_r) = render_export(&project);

    let (mon_peak_l, mon_peak_r) = (peak(&mon_l), peak(&mon_r));
    let (exp_peak_l, exp_peak_r) = (peak(&exp_l), peak(&exp_r));
    let ratio_l = f64::from(mon_peak_l) / f64::from(exp_peak_l);
    let ratio_r = f64::from(mon_peak_r) / f64::from(exp_peak_r);

    println!(
        "[#299/neutral-leg] 监听 L/R = {mon_peak_l:.9} / {mon_peak_r:.9}；\
         导出 L/R = {exp_peak_l:.9} / {exp_peak_r:.9}；\
         ratio L/R = {ratio_l:.9} / {ratio_r:.9}（期望 √2/2 = {CENTRE_GAIN:.9}，\
         = −3.0103 dB）"
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
    // 本判据钉的是**既有差**：比值必须仍是 −3.0103 dB。
    // ⚠ 下一票修好源侧中心口径时，这个断言会变红 —— 那是**正确的**，改期望值即可。
    assert!(
        (ratio_l - CENTRE_GAIN).abs() < 1e-6 && (ratio_r - CENTRE_GAIN).abs() < 1e-6,
        "中性声相下监听/导出的比必须仍是 √2/2（既有 −3 dB 缺陷）；实测 {ratio_l} / {ratio_r}"
    );
}
