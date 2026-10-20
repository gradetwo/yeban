//! 任务单 `#260`：**`TrackPan` 自动化不再依赖 `TrackVolume` 泳道**（监听腿的判据）。
//!
//! ## 被钉住的缺陷（`#259` 逐字实测）
//!
//! `EngineHost::publish_automation` 的声相段**嵌在**"该轨有开着的 `TrackVolume` 泳道"
//! 那个 `if let` **里面** ⇒ **只有声相泳道、没有音量泳道**的轨整段被 `continue` 跳过
//! ⇒ 监听时声相自动化**一条都不下发**。demo 工程的**贝斯轨**（slot 1）正好就是这种轨。
//!
//! `#259` 为了拿到"声相真的落地"的正对照，给贝斯轨**补了一条音量泳道**
//! （`tests/automation_unapplied.rs` 的 `insert_volume_lane`）—— 那是**绕过**缺陷，
//! 本单把它撤回。本文件用**公开 API**（没有夹具补全）钉住修好之后的行为。
//!
//! ## 本文件证明什么（与任务单 §2 的判据对应）
//!
//! | 判据 | 测试 |
//! | :--- | :--- |
//! | ① 只有声相泳道 ⇒ 修前 0 条 / 修后 2 条 | [`event_counts_follow_the_present_lanes`] |
//! | ② 同一轨发 2 条，且监听真的开始移动声相 | [`a_pan_only_lane_moves_the_monitor_pan`] |
//! | ③ 音量＋声相两条都有 ⇒ 3 条（正对照） | [`event_counts_follow_the_present_lanes`] ＋ `engine_host.rs` 的单测（逐值与顺序） |
//! | ④ 只有音量泳道 ⇒ 仍只 1 条 | [`event_counts_follow_the_present_lanes`] |
//! | ⑥ `unapplied` 计数不受影响 | [`unapplied_send_gain_counting_is_unchanged`] |
//! | ⑦ 离线导出修前/修后逐字节相同 | [`the_export_path_already_executed_pan_and_does_not_change`] |
//!
//! ## 边界（不许当成已证）
//!
//! - ⛔ **没有人耳听**：判据证明的是"L/R 峰值在首段/末段反向"＋"监听输出字节变了"，
//!   不是听感。
//! - ⛔ **生产窗口 `run_gui` 的事件循环没有执行**：本文件驱动的是它调用的**同一个**
//!   `EngineHost`（`reload` / `publish_automation` / `render_into`）。
//! - ⚠ 离线导出（`--export-wav`）走的是**另一条**执行路径
//!   （`yeban_render::automation::TrackPanAutomation`，`#257`）⇒ 本单**不改**它的字节；
//!   判据 ⑦ 就是这一条的正对照。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::str::FromStr as _;
use std::sync::atomic::{AtomicU64, Ordering};

use yeban_app::engine_host::{EngineHost, unapplied_automation_lanes};
use yeban_app::export_wav::{WavExportOptions, export_wav};
use yeban_engine::block::DEFAULT_BLOCK_FRAMES;
use yeban_model::{
    AssetHash, AutomationLane, AutomationPoint, AutomationTarget, AutomationWriteMode, ClipContent,
    ClipPlacement, ClipPoolEntry, CurveType, EntityId, LoopConfig, MidiNote, RoutingEdge,
    RoutingKind, TrackKind, TrackV3, YebanProjectV1,
};
use yeban_render::dither::BitDepth;

const FRAMES: u64 = 48_000;
/// 1 秒 = 1920 tick（120 BPM / 960 PPQ）⇒ 375 个量子 × 128 帧。
const QUANTA: usize = 375;

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn id(text: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8{text}")).expect("夹具 ULID 必须合法")
}

/// 临时目录（`Drop` 删掉自己；工作树里不留临时文件）。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-260-{tag}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        Self { dir }
    }

    fn wav(&self, tag: &str) -> PathBuf {
        self.dir.join(format!("{tag}.wav"))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn automation_point(tick: u64, value: f32) -> AutomationPoint {
    AutomationPoint {
        id: EntityId::new(),
        tick,
        value,
        curve: CurveType::Linear,
    }
}

/// 一串点 → 泳道的 `points` 表（**键 = 点自己的 `id`**；键与内嵌 id 不一致会被 `validate()` 拒）。
fn automation_points(points: &[(u64, f32)]) -> BTreeMap<EntityId, AutomationPoint> {
    points
        .iter()
        .map(|(tick, value)| {
            let point = automation_point(*tick, *value);
            (point.id, point)
        })
        .collect()
}

/// `Master` ＋ 一条 MIDI 轨（一个 1920 tick = 1 秒的音符）。
///
/// `pan_lane` / `volume_lane` 决定这条轨上挂哪些**开着**的泳道 ——
/// **没有别的泳道**，"只有声相泳道"就是字面意思。
fn fixture(pan_lane: bool, volume_lane: bool) -> (YebanProjectV1, EntityId) {
    let master = id("M001");
    let track = id("T001");
    let clip = id("K001");
    let placement = id("P001");
    let note = id("N001");
    let edge = id("E001");

    let mut project = YebanProjectV1 {
        id: id("Z001"),
        title: "pan monitor fixture".to_owned(),
        bpm: 120.0,
        rng_seed: 0x5945_4241_4E00_0260,
        master_bus_track_id: master,
        ..YebanProjectV1::default()
    };
    project.tracks.insert(
        master,
        TrackV3 {
            id: master,
            name: "Master".to_owned(),
            kind: TrackKind::Master,
            ..TrackV3::default()
        },
    );
    let mut lead = TrackV3 {
        id: track,
        name: "Lead".to_owned(),
        kind: TrackKind::Midi,
        volume_db: 0.0,
        ..TrackV3::default()
    };
    lead.clips.insert(
        placement,
        ClipPlacement {
            id: placement,
            clip_id: clip,
            start_tick: 0,
            duration_ticks: 1_920,
            loop_config: LoopConfig::default(),
            muted: false,
        },
    );
    project.tracks.insert(track, lead);
    project.clip_pool.insert(
        clip,
        ClipPoolEntry {
            id: clip,
            name: "Clip".to_owned(),
            content: ClipContent::Midi {
                notes: BTreeMap::from([(
                    note,
                    MidiNote {
                        velocity: 100,
                        ..MidiNote::new(note, 0, 69, 1_920)
                    },
                )]),
            },
        },
    );
    project.routing_graph.nodes = vec![track, master];
    project.routing_graph.edges.insert(
        edge,
        RoutingEdge {
            id: edge,
            source_node: track,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );

    if pan_lane {
        let target = AutomationTarget::TrackPan { track_id: track };
        project
            .tracks
            .get_mut(&track)
            .expect("音轨必须在")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    // -1 → 0 → +1：首段全左、末段全右（等功率曲线的两个端点）。
                    points: automation_points(&[(0, -1.0), (960, 0.0), (1_920, 1.0)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Write,
                    domain: None,
                },
            );
    }
    if volume_lane {
        let target = AutomationTarget::TrackVolume { track_id: track };
        project
            .tracks
            .get_mut(&track)
            .expect("音轨必须在")
            .automation_lanes
            .insert(
                target,
                AutomationLane {
                    target,
                    points: automation_points(&[(0, 0.0), (1_920, 0.0)]),
                    read_enabled: true,
                    write_mode: AutomationWriteMode::Write,
                    domain: None,
                },
            );
    }

    project.validate().expect("夹具工程必须合法");
    (project, track)
}

/// 建一代引擎并**按生产形态**每跳传工程（`Some`）⇒ 返回这一跳实际写入的条数。
fn published_with_engine(project: &YebanProjectV1) -> usize {
    let mut host = EngineHost::new();
    host.reload(project, 0).expect("引擎重建");
    host.publish_automation(Some(project))
}

/// 一次监听渲染的读数。
struct LiveRender {
    samples: Vec<f32>,
    channels: u16,
    /// 每一跳 `publish_automation` 的返回值（写入 SPSC 的条数）。
    written: Vec<usize>,
}

/// 走生产同一条腿：`reload` → 每跳 `publish_automation` → `render_into`（一个量子）。
fn live_render(project: &YebanProjectV1, quanta: usize) -> LiveRender {
    let mut host = EngineHost::new();
    host.reload(project, 0).expect("引擎重建");
    let channels = host
        .current_snapshot()
        .expect("有引擎必有快照")
        .channels()
        .max(1);
    let block = DEFAULT_BLOCK_FRAMES * usize::from(channels);
    let mut samples = vec![0.0_f32; block * quanta];
    let mut written = Vec::with_capacity(quanta);
    for index in 0..quanta {
        written.push(host.publish_automation(Some(project)));
        let driven = host.render_into(
            &mut samples[index * block..(index + 1) * block],
            channels,
            1,
        );
        assert_eq!(driven, 1, "控制腿必须推进恰好 1 个量子");
    }
    LiveRender {
        samples,
        channels,
        written,
    }
}

/// 某一跳区间里某个声道的**峰值**（帧区间是 `[start, start + frames)`）。
fn channel_peak(
    samples: &[f32],
    channels: u16,
    channel: usize,
    start: usize,
    frames: usize,
) -> f32 {
    let stride = usize::from(channels);
    let total_frames = samples.len() / stride;
    let mut peak = 0.0_f32;
    for frame in start..(start + frames).min(total_frames) {
        peak = peak.max(samples[frame * stride + channel].abs());
    }
    peak
}

/// 浮点样本的 `sha256`（按小端字节逐位哈希 —— 判"音频字节有没有变"）。
fn sha_f32(samples: &[f32]) -> String {
    let mut bytes = Vec::with_capacity(samples.len() * 4);
    for sample in samples {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    AssetHash::of_bytes(&bytes).as_str().to_owned()
}

fn export_options() -> WavExportOptions {
    WavExportOptions {
        frames: FRAMES,
        sample_rate: 48_000,
        channels: 2,
        bit_depth: BitDepth::Int24,
        ..WavExportOptions::default()
    }
}

/// `--export-wav` 一次并返回产物的 `sha256`。
fn export_sha(scratch: &Scratch, tag: &str, project: &YebanProjectV1) -> String {
    let path = scratch.wav(tag);
    let report = export_wav(project, &path, export_options()).expect("--export-wav 必须成功");
    assert_eq!(report.frames, FRAMES);
    let bytes = std::fs::read(&path).expect("读产物");
    AssetHash::of_bytes(&bytes).as_str().to_owned()
}

/// 判据 ① / ③ / ④：**条数跟着"有哪些泳道"走**，而不是跟着"有没有音量泳道"走。
#[test]
fn event_counts_follow_the_present_lanes() {
    let (pan_only, _) = fixture(true, false);
    let (volume_only, _) = fixture(false, true);
    let (both, _) = fixture(true, true);

    let pan_only_written = published_with_engine(&pan_only);
    let volume_only_written = published_with_engine(&volume_only);
    let both_written = published_with_engine(&both);
    println!(
        "### #260 每跳条数：只有声相={pan_only_written} 只有音量={volume_only_written} 两条都有={both_written}"
    );

    assert_eq!(
        pan_only_written, 2,
        "修前这里是 0（只有声相泳道 ⇒ 声相段被音量分支跳过）"
    );
    assert_eq!(volume_only_written, 1, "只有音量 ⇒ 仍只 1 条（⛔ 不多发）");
    assert_eq!(both_written, 3, "正对照：音量 1 ＋ 声相 2");
}

/// 判据 ② / ⑦：**监听真的开始移动声相**，且离线导出**不变**。
///
/// * 监听：同一份"只有声相泳道"的工程，开/关泳道两条路径的输出 `sha256` 必须**不同**
///   —— 修前两者**逐位相同**（都发 0 条）。
/// * 导出：离线路径（`#257`）本来就执行声相 ⇒ 开泳道与关泳道的导出**不同**是**前提**，
///   而本单不改它（`#260` 的读数由注入前后的同一条导出给出）。
#[test]
fn a_pan_only_lane_moves_the_monitor_pan() {
    let (project, _track) = fixture(true, false);
    let mut muted = project.clone();
    // 关掉声相泳道（但**保留**泳道本身：`read_enabled:false` 与"没有泳道"同义）。
    for track in muted.tracks.values_mut() {
        for lane in track.automation_lanes.values_mut() {
            if matches!(lane.target, AutomationTarget::TrackPan { .. }) {
                lane.read_enabled = false;
            }
        }
    }

    let enabled = live_render(&project, QUANTA);
    let disabled = live_render(&muted, QUANTA);

    // 1 秒 = 4 段 × 12000 帧：首段声相在 [-1, -0.5] ⇒ 左重；末段在 [0.5, 1] ⇒ 右重。
    let quarter = 12_000;
    let head_left = channel_peak(&enabled.samples, enabled.channels, 0, 0, quarter);
    let head_right = channel_peak(&enabled.samples, enabled.channels, 1, 0, quarter);
    let tail_left = channel_peak(&enabled.samples, enabled.channels, 0, 3 * quarter, quarter);
    let tail_right = channel_peak(&enabled.samples, enabled.channels, 1, 3 * quarter, quarter);
    let enabled_sha = sha_f32(&enabled.samples);
    let disabled_sha = sha_f32(&disabled.samples);

    // 判据 ⑦：离线导出（另一条执行路径）**已经**执行声相 —— 与"有没有音量泳道"无关。
    let scratch = Scratch::new("export");
    let enabled_export = export_sha(&scratch, "pan", &project);
    let disabled_export = export_sha(&scratch, "no-pan", &muted);

    // 先把**全部**读数打出来，再断言（修前的失败运行也要留下读数，别让 panic 挡住）。
    println!(
        "### #260 每跳条数：开泳道首跳={:?} 关泳道首跳={:?}",
        enabled.written.first(),
        disabled.written.first()
    );
    println!(
        "### #260 监听峰值：首段 L={head_left:.6} R={head_right:.6}；末段 L={tail_left:.6} R={tail_right:.6}"
    );
    println!("### #260 监听 sha256（开/关泳道）= {enabled_sha} / {disabled_sha}");
    println!("### #260 导出 sha256（开/关泳道）= {enabled_export} / {disabled_export}");

    assert!(
        enabled.written.iter().all(|written| *written == 2),
        "每跳必须写 2 条（左+右），实得首跳 {:?}",
        enabled.written.first()
    );
    assert!(
        disabled.written.iter().all(|written| *written == 0),
        "关掉的声相泳道不得下发"
    );
    assert!(
        head_left > head_right,
        "首段声相 ≈ -1（硬左）⇒ 左声道必须更响（L={head_left} R={head_right}）"
    );
    assert!(
        tail_right > tail_left,
        "末段声相 ≈ +1（硬右）⇒ 右声道必须更响（L={tail_left} R={tail_right}）"
    );
    assert_ne!(
        enabled_sha, disabled_sha,
        "修前开启泳道的监听输出与关掉**逐位相同**（一条都没下发）；修后必须不同"
    );
    assert_ne!(
        enabled_export, disabled_export,
        "离线导出本来就执行声相自动化（#257）⇒ 开/关必须不同"
    );
}

/// 判据 ⑥：`unapplied` 的读数（`SendGain` 那几列）与声相采样**互不影响**。
#[test]
fn unapplied_send_gain_counting_is_unchanged() {
    let (mut project, track) = fixture(true, false);
    let edge = project
        .routing_graph
        .edges
        .keys()
        .next()
        .copied()
        .expect("夹具必须有路由边");
    let target = AutomationTarget::SendGain {
        track_id: track,
        edge_id: edge,
    };
    project
        .tracks
        .get_mut(&track)
        .expect("音轨必须在")
        .automation_lanes
        .insert(
            target,
            AutomationLane {
                target,
                points: automation_points(&[(0, -6.0)]),
                read_enabled: true,
                write_mode: AutomationWriteMode::Off,
                domain: None,
            },
        );
    project.validate().expect("加泳道后夹具仍须合法");

    assert_eq!(unapplied_automation_lanes(&project).send_gain, 1);
    assert_eq!(published_with_engine(&project), 2, "SendGain 不产生事件");

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("引擎重建");
    assert_eq!(host.publish_automation(Some(&project)), 2);
    assert_eq!(host.automation_unapplied(), 1);
    assert_eq!(host.automation_unapplied_breakdown().send_gain, 1);
    assert_eq!(host.automation_unapplied_breakdown().device_param, 0);
    assert_eq!(host.automation_unapplied_breakdown().macro_lanes, 0);
    assert_eq!(host.automation_counts(), (2, 0));
}
